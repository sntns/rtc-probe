//! Linux's I2C buses: `/dev/i2c-N` (the `i2c-dev` module), read with
//! `I2C_RDWR`, and the devices the kernel instantiated on them
//! (`/sys/bus/i2c/devices`), which rtc-probe can add with `new_device`.
//!
//! `I2C_RDWR` reads a device whether a kernel driver is bound to it or not:
//! a chip already handled by a driver can still be identified.

use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::path::PathBuf;

use crate::bus::BusError;
use crate::bus::I2cBus;

const I2C_RDWR: libc::c_ulong = 0x0707;
const I2C_M_RD: u16 = 0x0001;

/// `struct i2c_msg` (linux/i2c.h).
#[repr(C)]
struct I2cMsg {
    addr: u16,
    flags: u16,
    len: u16,
    buf: *mut u8,
}

/// `struct i2c_rdwr_ioctl_data` (linux/i2c-dev.h).
#[repr(C)]
struct I2cRdwrData {
    msgs: *mut I2cMsg,
    nmsgs: u32,
}

/// `/dev/i2c-N`.
pub struct LinuxI2c {
    file: File,
}

impl LinuxI2c {
    /// Opens bus `bus` (`/dev/i2c-<bus>`). Fails when the bus does not exist
    /// or `i2c-dev` is not loaded.
    pub fn open(bus: u32) -> io::Result<Self> {
        Self::open_path(format!("/dev/i2c-{bus}"))
    }

    /// Opens an I2C character device by its path.
    pub fn open_path(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        Ok(Self { file })
    }
}

impl I2cBus for LinuxI2c {
    fn read(&mut self, address: u8, register: u8, buf: &mut [u8]) -> Result<(), BusError> {
        let len = u16::try_from(buf.len())
            .map_err(|_| BusError::Io(io::Error::from(io::ErrorKind::InvalidInput)))?;
        let mut pointer = register;
        let mut msgs = [
            I2cMsg {
                addr: u16::from(address),
                flags: 0,
                len: 1,
                buf: &mut pointer,
            },
            I2cMsg {
                addr: u16::from(address),
                flags: I2C_M_RD,
                len,
                buf: buf.as_mut_ptr(),
            },
        ];
        let mut data = I2cRdwrData {
            msgs: msgs.as_mut_ptr(),
            nmsgs: 2,
        };
        // SAFETY: I2C_RDWR reads `nmsgs` `struct i2c_msg` from `msgs`, and
        // writes at most `len` bytes to each read message's `buf`: both
        // point to live buffers of those lengths for the whole call.
        let ret = unsafe { libc::ioctl(self.file.as_raw_fd(), I2C_RDWR as _, &mut data) };
        if ret < 0 {
            let error = io::Error::last_os_error();
            return Err(match error.raw_os_error() {
                // What the adapters report when the address is not
                // acknowledged (i2c-bcm2835: EREMOTEIO; others ENXIO).
                Some(libc::ENXIO | libc::EREMOTEIO) => BusError::Absent,
                _ => BusError::Io(error),
            });
        }
        Ok(())
    }
}

/// A device the kernel instantiated on a bus (`/sys/bus/i2c/devices/N-00AA`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelDevice {
    /// Where it is.
    pub address: u8,
    /// The name it was instantiated with (`ds1307`, `pcf2127`, ...).
    pub name: String,
    /// The driver bound to it, if any.
    pub driver: Option<String>,
}

/// The devices the kernel knows on a bus, and the means to add or remove
/// one.
#[derive(Debug, Clone)]
pub struct Sysfs {
    root: PathBuf,
    bus: u32,
}

impl Sysfs {
    /// Bus `bus` under `/sys/bus/i2c/devices`.
    pub fn new(bus: u32) -> Self {
        Self::with_root("/sys/bus/i2c/devices", bus)
    }

    /// Bus `bus` under another root (tests).
    pub fn with_root(root: impl Into<PathBuf>, bus: u32) -> Self {
        Self {
            root: root.into(),
            bus,
        }
    }

    fn device_dir(&self, address: u8) -> PathBuf {
        self.root.join(format!("{}-{:04x}", self.bus, address))
    }

    /// The device the kernel instantiated at `address`, if any.
    pub fn device(&self, address: u8) -> Option<KernelDevice> {
        let dir = self.device_dir(address);
        let name = std::fs::read_to_string(dir.join("name")).ok()?;
        let driver = std::fs::read_link(dir.join("driver"))
            .ok()
            .and_then(|link| link.file_name().map(|n| n.to_string_lossy().into_owned()));
        Some(KernelDevice {
            address,
            name: name.trim().to_string(),
            driver,
        })
    }

    /// Asks the kernel to instantiate device `name` at `address`
    /// (`new_device`): its driver is loaded and bound by the kernel.
    pub fn new_device(&self, name: &str, address: u8) -> io::Result<()> {
        let path = self
            .root
            .join(format!("i2c-{}", self.bus))
            .join("new_device");
        std::fs::write(path, format!("{name} 0x{address:02x}\n"))
    }

    /// Removes the device the kernel instantiated at `address`
    /// (`delete_device`): only one added with `new_device` can be.
    pub fn delete_device(&self, address: u8) -> io::Result<()> {
        let path = self
            .root
            .join(format!("i2c-{}", self.bus))
            .join("delete_device");
        std::fs::write(path, format!("0x{address:02x}\n"))
    }
}

/// What binding a chip did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bound {
    /// The kernel had already bound a driver there: left as is.
    Already(KernelDevice),
    /// Instantiated, and its driver bound.
    Bound(KernelDevice),
}

/// Why a chip was not bound.
#[derive(Debug)]
pub enum BindError {
    /// The kernel declares a device there with no driver bound (a
    /// device-tree node whose driver failed): `new_device` can't replace it.
    Declared(KernelDevice),
    /// `new_device` failed.
    NewDevice(io::Error),
    /// Instantiated, but no driver bound in time.
    NoDriver,
}

impl std::fmt::Display for BindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BindError::Declared(device) => write!(
                f,
                "the kernel already declares `{}` there, with no driver bound (a device-tree node?): remove it",
                device.name
            ),
            BindError::NewDevice(e) => write!(f, "new_device failed: {e}"),
            BindError::NoDriver => f.write_str("instantiated, but no driver bound to it"),
        }
    }
}

impl std::error::Error for BindError {}

impl Sysfs {
    /// Binds `device` at `address`, unless the kernel already has a device
    /// there: `load` loads its module first (without udev, nothing loads it
    /// on `new_device`), then the driver is waited for up to `wait`.
    pub fn bind(
        &self,
        device: &str,
        address: u8,
        load: impl FnOnce(),
        wait: std::time::Duration,
    ) -> Result<Bound, BindError> {
        if let Some(existing) = self.device(address) {
            return if existing.driver.is_some() {
                Ok(Bound::Already(existing))
            } else {
                Err(BindError::Declared(existing))
            };
        }
        load();
        self.new_device(device, address)
            .map_err(BindError::NewDevice)?;
        let step = std::time::Duration::from_millis(50);
        let mut waited = std::time::Duration::ZERO;
        loop {
            if let Some(bound) = self.device(address).filter(|d| d.driver.is_some()) {
                return Ok(Bound::Bound(bound));
            }
            if waited >= wait {
                return Err(BindError::NoDriver);
            }
            std::thread::sleep(step);
            waited += step;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kernel_device_and_its_driver_are_read_from_sysfs() {
        let root = std::env::temp_dir().join(format!("rtc-probe-sysfs-{}", std::process::id()));
        let dev = root.join("1-0051");
        std::fs::create_dir_all(&dev).unwrap();
        std::fs::write(dev.join("name"), "pcf2127\n").unwrap();
        let drivers = root.join("drivers/rtc-pcf2127-i2c");
        std::fs::create_dir_all(&drivers).unwrap();
        let _ = std::fs::remove_file(dev.join("driver"));
        std::os::unix::fs::symlink(&drivers, dev.join("driver")).unwrap();
        std::fs::create_dir_all(root.join("1-0068")).unwrap();
        std::fs::write(root.join("1-0068/name"), "ds1307\n").unwrap();

        let sysfs = Sysfs::with_root(&root, 1);

        assert_eq!(
            sysfs.device(0x51),
            Some(KernelDevice {
                address: 0x51,
                name: "pcf2127".into(),
                driver: Some("rtc-pcf2127-i2c".into()),
            })
        );
        assert_eq!(sysfs.device(0x68).unwrap().driver, None);
        assert_eq!(sysfs.device(0x6f), None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn new_device_writes_the_name_and_the_address() {
        let root = std::env::temp_dir().join(format!("rtc-probe-new-{}", std::process::id()));
        std::fs::create_dir_all(root.join("i2c-1")).unwrap();
        let sysfs = Sysfs::with_root(&root, 1);

        sysfs.new_device("ds3231", 0x68).unwrap();

        assert_eq!(
            std::fs::read_to_string(root.join("i2c-1/new_device")).unwrap(),
            "ds3231 0x68\n"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}

#[cfg(test)]
mod bind_tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    /// An empty sysfs with bus 1, removed when dropped.
    struct Root(PathBuf);

    impl Root {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "rtc-probe-{name}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("i2c-1")).unwrap();
            Root(root)
        }

        /// The kernel instantiating `name` at `address`, with a driver
        /// bound or not.
        fn device(&self, address: u8, name: &str, driver: Option<&str>) {
            let dir = self.0.join(format!("1-{address:04x}"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("name"), format!("{name}\n")).unwrap();
            if let Some(driver) = driver {
                let target = self.0.join("drivers").join(driver);
                std::fs::create_dir_all(&target).unwrap();
                std::os::unix::fs::symlink(&target, dir.join("driver")).unwrap();
            }
        }

        fn sysfs(&self) -> Sysfs {
            Sysfs::with_root(&self.0, 1)
        }
    }

    impl Drop for Root {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_device_already_bound_is_left_alone() {
        let root = Root::new("already");
        root.device(0x51, "pcf2127", Some("rtc-pcf2127-i2c"));
        let loaded = AtomicBool::new(false);

        let bound = root
            .sysfs()
            .bind(
                "pcf2129",
                0x51,
                || loaded.store(true, Ordering::SeqCst),
                Duration::ZERO,
            )
            .unwrap();

        assert!(matches!(bound, Bound::Already(d) if d.name == "pcf2127"));
        assert!(!loaded.load(Ordering::SeqCst));
        assert!(!root.0.join("i2c-1/new_device").exists());
    }

    #[test]
    fn a_device_declared_without_a_driver_is_not_replaced() {
        let root = Root::new("declared");
        root.device(0x68, "ds1307", None);

        let error = root
            .sysfs()
            .bind("ds3231", 0x68, || {}, Duration::ZERO)
            .unwrap_err();

        assert!(matches!(error, BindError::Declared(d) if d.name == "ds1307"));
        assert!(!root.0.join("i2c-1/new_device").exists());
    }

    #[test]
    fn a_chip_is_loaded_then_instantiated_then_waited_for() {
        let root = Root::new("bind");
        let loaded = AtomicBool::new(false);
        let sysfs = root.sysfs();
        let kernel = {
            let dir = root.0.clone();
            std::thread::spawn(move || {
                // The kernel, once new_device was written.
                let request = dir.join("i2c-1/new_device");
                while !request.exists() {
                    std::thread::sleep(Duration::from_millis(10));
                }
                let device = dir.join("1-0068");
                std::fs::create_dir_all(&device).unwrap();
                std::fs::write(device.join("name"), "ds3231\n").unwrap();
                let driver = dir.join("drivers/rtc-ds1307");
                std::fs::create_dir_all(&driver).unwrap();
                std::os::unix::fs::symlink(&driver, device.join("driver")).unwrap();
            })
        };

        let bound = sysfs
            .bind(
                "ds3231",
                0x68,
                || loaded.store(true, Ordering::SeqCst),
                Duration::from_secs(5),
            )
            .unwrap();
        kernel.join().unwrap();

        assert!(loaded.load(Ordering::SeqCst));
        assert_eq!(
            std::fs::read_to_string(root.0.join("i2c-1/new_device")).unwrap(),
            "ds3231 0x68\n"
        );
        assert!(matches!(bound, Bound::Bound(d) if d.driver.as_deref() == Some("rtc-ds1307")));
    }

    #[test]
    fn a_chip_no_driver_takes_is_reported() {
        let root = Root::new("nodriver");

        let error = root
            .sysfs()
            .bind("ds3231", 0x68, || {}, Duration::from_millis(100))
            .unwrap_err();

        assert!(matches!(error, BindError::NoDriver));
    }

    #[test]
    fn a_bus_the_kernel_lacks_fails_new_device() {
        let root = Root::new("nobus");
        let sysfs = Sysfs::with_root(&root.0, 7);

        let error = sysfs
            .bind("ds3231", 0x68, || {}, Duration::ZERO)
            .unwrap_err();

        assert!(matches!(error, BindError::NewDevice(_)));
    }

    #[test]
    fn a_missing_bus_device_cannot_be_opened() {
        assert!(LinuxI2c::open_path("/nonexistent/i2c-42").is_err());
    }
}
