//! `rtc-probe`: identify the RTC on an I2C bus, and bind its driver.

use std::process::ExitCode;
use std::time::Duration;

use rtc_probe::identify::Confidence;
use rtc_probe::identify::Finding;
use rtc_probe::identify::Identification;
use rtc_probe::identify::Oscillator;
use rtc_probe::linux::LinuxI2c;
use rtc_probe::linux::Sysfs;

const USAGE: &str = "\
Usage: rtc-probe [--bus N] [--bind] [--dump]

Identifies the real-time clock on an I2C bus by reading its registers
(nothing is written to it), and binds its Linux driver with --bind.

  --bus N   the bus to probe (/dev/i2c-N, default 1); needs i2c-dev
  --bind    instantiate the chip found (new_device), after loading its
            module; only a chip identified as certain or probable
  --dump    also print the registers read
  --help    this help

Exit status: 0 an RTC was identified (and bound with --bind), 1 none was,
2 an error, an ambiguous chip or a failed bind.
";

struct Options {
    bus: u32,
    bind: bool,
    dump: bool,
}

fn options() -> Result<Options, String> {
    let mut options = Options {
        bus: 1,
        bind: false,
        dump: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bus" => {
                let value = args.next().ok_or("--bus needs a number")?;
                options.bus = value.parse().map_err(|_| format!("bad bus `{value}`"))?;
            }
            "--bind" => options.bind = true,
            "--dump" => options.dump = true,
            "--help" | "-h" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "--version" | "-V" => {
                println!("rtc-probe {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    Ok(options)
}

fn describe(found: &Identification) -> String {
    let confidence = match found.confidence {
        Confidence::Certain => "certain",
        Confidence::Probable => "probable",
        Confidence::Possible => "possible",
    };
    let oscillator = match found.oscillator {
        Oscillator::Running => "oscillator running",
        Oscillator::Stopped => "oscillator stopped or flagged: time not trusted",
    };
    let wrap = found
        .wrap
        .map_or("no wrap seen".to_string(), |w| format!("{w} registers"));
    format!(
        "{} ({confidence}; {wrap}; {oscillator}), driver {} ({})",
        found.chip, found.device, found.module
    )
}

/// Loads `module`: without udev (an initramfs), nothing loads it on
/// `new_device`.
fn modprobe(module: &str) {
    match std::process::Command::new("modprobe").arg(module).status() {
        Ok(status) if status.success() => {}
        Ok(status) => eprintln!("rtc-probe: modprobe {module}: {status}"),
        Err(e) => eprintln!("rtc-probe: modprobe {module}: {e}"),
    }
}

/// Binds the chip found, unless the kernel already has a device there.
fn bind(sysfs: &Sysfs, found: &Identification) -> Result<String, String> {
    let address = found.address;
    if let Some(device) = sysfs.device(address) {
        return match device.driver {
            Some(driver) => Ok(format!("already bound: {} ({driver})", device.name)),
            None => Err(format!(
                "the kernel already declares `{}` there, with no driver bound (a device-tree node?): remove it to bind {}",
                device.name, found.device
            )),
        };
    }
    modprobe(found.module);
    sysfs
        .new_device(found.device, address)
        .map_err(|e| format!("new_device {} 0x{address:02x}: {e}", found.device))?;
    for _ in 0..20 {
        if let Some(device) = sysfs.device(address) {
            if let Some(driver) = device.driver {
                return Ok(format!("bound: {} ({driver})", device.name));
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(format!(
        "{} instantiated at 0x{address:02x}, but no driver bound to it",
        found.device
    ))
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .chunks(16)
        .enumerate()
        .map(|(i, row)| {
            let row: Vec<String> = row.iter().map(|b| format!("{b:02x}")).collect();
            format!("    {:03x}: {}", i * 16, row.join(" "))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn main() -> ExitCode {
    let options = match options() {
        Ok(options) => options,
        Err(e) => {
            eprintln!("rtc-probe: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let mut i2c = match LinuxI2c::open(options.bus) {
        Ok(i2c) => i2c,
        Err(e) => {
            eprintln!(
                "rtc-probe: /dev/i2c-{}: {e} (is i2c-dev loaded?)",
                options.bus
            );
            return ExitCode::from(2);
        }
    };
    let sysfs = Sysfs::new(options.bus);
    let addresses = rtc_probe::chips::addresses();
    let observations = rtc_probe::identify::observe(&mut i2c, &addresses, || {
        std::thread::sleep(rtc_probe::identify::TICK)
    });

    let mut found = Vec::new();
    let mut failed = false;
    for (address, observation) in observations {
        let prefix = format!("i2c-{} 0x{address:02x}", options.bus);
        let kernel = sysfs
            .device(address)
            .map(|d| match d.driver {
                Some(driver) => format!(" [kernel: {} bound to {driver}]", d.name),
                None => format!(" [kernel: {}, no driver bound]", d.name),
            })
            .unwrap_or_default();
        let observation = match observation {
            Ok(observation) => observation,
            Err(rtc_probe::bus::BusError::Absent) => continue,
            Err(e) => {
                println!("{prefix}: {e}{kernel}");
                failed = true;
                continue;
            }
        };
        match rtc_probe::identify::identify(&observation) {
            Finding::Absent => {}
            Finding::Unknown { wrap, .. } => {
                let wrap = wrap.map_or("no known wrap".to_string(), |w| format!("{w} registers"));
                println!("{prefix}: a device answers, no known RTC ({wrap}){kernel}");
            }
            Finding::Chip(identification) => {
                println!("{prefix}: {}{kernel}", describe(&identification));
                found.push(identification);
            }
            Finding::Ambiguous { candidates, .. } => {
                let names: Vec<&str> = candidates.iter().map(|c| c.chip).collect();
                println!(
                    "{prefix}: ambiguous, could be {}{kernel}",
                    names.join(" or ")
                );
                failed = true;
            }
        }
        if options.dump {
            println!("  first read:\n{}", hex(&observation.first[..64]));
            println!("  1.1 s later:\n{}", hex(&observation.second[..64]));
        }
    }

    if found.is_empty() {
        if !failed {
            println!("i2c-{}: no RTC found", options.bus);
        }
        return ExitCode::from(if failed { 2 } else { 1 });
    }
    if options.bind {
        for identification in &found {
            let prefix = format!("i2c-{} 0x{:02x}", options.bus, identification.address);
            if identification.confidence == Confidence::Possible {
                println!(
                    "{prefix}: not bound: only possible (its drivers write to the chip at probe)"
                );
                failed = true;
                continue;
            }
            match bind(&sysfs, identification) {
                Ok(message) => println!("{prefix}: {message}"),
                Err(message) => {
                    println!("{prefix}: {message}");
                    failed = true;
                }
            }
        }
    }
    if failed {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}
