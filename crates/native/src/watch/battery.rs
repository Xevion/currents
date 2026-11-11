//! Whether a Linux machine runs on battery, from the power supplies the kernel lists.

use std::fs;
use std::io;
use std::path::Path;

/// Where the kernel lists power supplies.
pub(crate) const SUPPLIES: &str = "/sys/class/power_supply";

fn read(dir: &Path, name: &str) -> String {
    fs::read_to_string(dir.join(name)).map(|s| s.trim().to_owned()).unwrap_or_default()
}

/// True only when the machine has a mains supply and none is plugged in. A machine with no
/// mains supply listed, as most desktops are, runs on mains. Batteries that power a device
/// rather than the machine, like a mouse's, say so in `scope` and are skipped.
pub(crate) fn on_battery(supplies: &Path) -> io::Result<bool> {
    let entries = match fs::read_dir(supplies) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    let (mut mains, mut plugged, mut discharging) = (false, false, false);
    for entry in entries {
        let dir = entry?.path();
        if read(&dir, "scope") == "Device" {
            continue;
        }
        match read(&dir, "type").as_str() {
            "Mains" | "USB" | "USB_C" | "USB_PD" => {
                mains = true;
                plugged |= read(&dir, "online") == "1";
            },
            "Battery" => discharging |= read(&dir, "status") == "Discharging",
            _ => {},
        }
    }
    // Without a mains supply to ask, a discharging battery is the only sign.
    Ok(if mains { !plugged } else { discharging })
}

#[cfg(test)]
mod tests {
    use assert2::check;
    use rstest::rstest;

    use super::*;

    /// A power supply directory, as `name` with each `file=value` written beside it.
    type Supply = (&'static str, &'static [(&'static str, &'static str)]);

    const AC_ON: Supply = ("AC", &[("type", "Mains"), ("online", "1")]);
    const AC_OFF: Supply = ("AC", &[("type", "Mains"), ("online", "0")]);
    const USB_C_ON: Supply = ("ucsi-source-psy-USBC000:001", &[("type", "USB"), ("online", "1")]);
    const BAT_CHARGING: Supply = ("BAT0", &[("type", "Battery"), ("status", "Charging")]);
    const BAT_DISCHARGING: Supply = ("BAT0", &[("type", "Battery"), ("status", "Discharging")]);
    const MOUSE: Supply = ("hidpp_battery_0", &[("type", "Battery"), ("scope", "Device"), ("status", "Discharging")]);
    const UPS: Supply = ("ups", &[("type", "UPS"), ("online", "0")]);

    #[rstest]
    #[case::laptop_plugged(&[AC_ON, BAT_CHARGING], false)]
    #[case::laptop_unplugged(&[AC_OFF, BAT_DISCHARGING], true)]
    #[case::laptop_on_usb_c(&[AC_OFF, USB_C_ON, BAT_CHARGING], false)]
    #[case::desktop(&[], false)]
    #[case::desktop_with_a_wireless_mouse(&[MOUSE], false)]
    #[case::desktop_on_a_ups(&[UPS], false)]
    #[case::battery_without_a_mains_entry(&[BAT_DISCHARGING], true)]
    fn battery_power_is_read_from_the_supplies(#[case] supplies: &[Supply], #[case] expected: bool) {
        let root = std::env::temp_dir().join(format!("currents-supplies-{}", std::process::id()));
        let root = root.join(format!("{:x}", supplies.as_ptr() as usize));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        for (name, files) in supplies {
            let dir = root.join(name);
            fs::create_dir_all(&dir).unwrap();
            for (file, value) in *files {
                fs::write(dir.join(file), format!("{value}\n")).unwrap();
            }
        }
        check!(on_battery(&root).unwrap() == expected);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_machine_without_the_supplies_directory_runs_on_mains() {
        check!(!on_battery(Path::new("/nonexistent/power_supply")).unwrap());
    }
}
