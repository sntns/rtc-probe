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
