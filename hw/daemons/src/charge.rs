//! Charging while "off": what `nibos-charge-boot` decides.
//!
//! Plugging a charger into a switched-off device switches it on: the PMIC does that by itself,
//! and records why in a register (the RK817's ON_SOURCE: one bit for the charger, one for the
//! power key). Booting all the way to the boot UI because a cable went in isn't what anyone
//! wants, so when the charger switched it on and is still plugged in, nibOS charges asleep
//! instead: the screens dark, the boot UI not started, the charge LED lit (it stays lit in
//! sleep), until someone asks for the device or takes the charger away:
//!
//! - **the power key, or opening the lid**: the normal boot carries on (the boot UI starts);
//! - **unplugging**: it powers off, as it was before the cable went in;
//! - **anything else that wakes it** (the charger's own interrupts, the RTC): it sleeps again.
//!
//! The board names the register in `NIBOS_POWER_ON_SOURCE` ([`OnSource`]).

/// Where the PMIC says what switched the device on: `NIBOS_POWER_ON_SOURCE="BUS ADDR REG
/// CHARGER KEY"`, an I2C bus number, the PMIC's address and register, and the register's bits
/// for "the charger was plugged in" and "the power key was pressed" (the RG DS+'s RK817:
/// `"0 0x20 0xf5 0x40 0x80"`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OnSource {
    pub bus: u32,
    pub addr: u16,
    pub reg: u8,
    pub charger_bit: u8,
    pub key_bit: u8,
}

impl OnSource {
    pub fn parse(text: &str) -> Result<OnSource, String> {
        let num = |s: &str| match s.strip_prefix("0x") {
            Some(hex) => u32::from_str_radix(hex, 16).ok(),
            None => s.parse().ok(),
        };
        let fields: Vec<u32> = text
            .split_whitespace()
            .map(|s| num(s).ok_or(format!("NIBOS_POWER_ON_SOURCE: {s} is not a number")))
            .collect::<Result<_, _>>()?;
        match fields[..] {
            [bus, addr @ ..=0x7f, reg @ ..=0xff, charger @ 1..=0xff, key @ 1..=0xff] => {
                Ok(OnSource {
                    bus,
                    addr: addr as u16,
                    reg: reg as u8,
                    charger_bit: charger as u8,
                    key_bit: key as u8,
                })
            }
            _ => Err(format!(
                "NIBOS_POWER_ON_SOURCE=\"{text}\" is not \"BUS ADDR REG CHARGER-BIT KEY-BIT\""
            )),
        }
    }

    /// Whether the register's value says the charger switched the device on (and the power key
    /// didn't).
    pub fn by_charger(&self, value: u8) -> bool {
        value & self.charger_bit != 0 && value & self.key_bit == 0
    }
}

/// What woke the device while it was charging asleep, as far as it matters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Woken {
    /// The power key was pressed.
    pub key: bool,
    /// The lid was opened.
    pub lid_opened: bool,
}

/// What to do after a wake.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// Carry on with the normal boot: start the boot UI.
    Boot,
    /// Power off: the charger is gone.
    PowerOff,
    /// Sleep again.
    Sleep,
}

/// After a wake: someone asked for the device, or the charger is gone, or neither.
pub fn next(woken: Woken, charger_online: bool) -> Next {
    if woken.key || woken.lid_opened {
        Next::Boot
    } else if !charger_online {
        Next::PowerOff
    } else {
        Next::Sleep
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RK817: OnSource = OnSource {
        bus: 0,
        addr: 0x20,
        reg: 0xf5,
        charger_bit: 0x40,
        key_bit: 0x80,
    };

    #[test]
    fn the_rg_ds_plus_register() {
        assert_eq!(OnSource::parse("0 0x20 0xf5 0x40 0x80"), Ok(RK817));
        assert!(OnSource::parse("0 0x20 0xf5 0x40").is_err());
        assert!(
            OnSource::parse("0 0x20 0xf5 0 0x80").is_err(),
            "no charger bit"
        );
        assert!(
            OnSource::parse("0 0x80 0xf5 0x40 0x80").is_err(),
            "not a 7-bit address"
        );
        assert!(OnSource::parse("0 0x20 0xzz 0x40 0x80").is_err());
    }

    #[test]
    fn only_the_charger_alone_is_a_charger_power_on() {
        assert!(RK817.by_charger(0x40));
        assert!(!RK817.by_charger(0x80), "the power key");
        assert!(!RK817.by_charger(0xc0), "the key too: someone wants it on");
        assert!(!RK817.by_charger(0x02), "a restart");
        assert!(!RK817.by_charger(0x00));
    }

    #[test]
    fn asked_for_boots_unplugged_powers_off_else_sleeps() {
        let key = Woken {
            key: true,
            lid_opened: false,
        };
        let lid = Woken {
            key: false,
            lid_opened: true,
        };
        let nothing = Woken::default();
        assert_eq!(next(key, true), Next::Boot);
        assert_eq!(next(lid, true), Next::Boot);
        assert_eq!(
            next(key, false),
            Next::Boot,
            "asked for, on battery: still boots"
        );
        assert_eq!(next(nothing, false), Next::PowerOff);
        assert_eq!(next(nothing, true), Next::Sleep);
    }
}
