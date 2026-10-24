//! `provision.toml`, the settings a downloadable image's user puts on the card's `NIBOS`
//! partition before (or after) the first boot: Wi-Fi, SSH keys, the user's password, the time
//! zone, which card holds the games (`docs/design/images.md`). `nibos-provision` reads it at
//! boot, checks it here, applies it, and wipes it; a file that fails a check is kept aside with
//! the reason, never the values, and the device boots without it.
//!
//! ```toml
//! timezone = "Europe/London"     # optional, a name under /usr/share/zoneinfo
//! data_card = "auto"             # optional: auto, tf1 or tf2
//!
//! [wifi]                         # optional
//! ssid = "My network"
//! password = "the passphrase"    # 8-63 characters; leave it out for an open network
//!
//! [user]                         # optional: the image's user, nibos
//! password = "..."               # leave it out for none (the console logs in by itself)
//!
//! [ssh]                          # optional: SSH is off until a key is given
//! authorized_keys = ["ssh-ed25519 AAAA... me@laptop"]
//! ```

use std::collections::BTreeMap;
use toml::Value;

/// What a valid file asks for. `None`: leave as it is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Provision {
    pub timezone: Option<String>,
    pub data_card: Option<DataCard>,
    pub wifi: Option<Wifi>,
    /// `Some(None)`: no password (asked for by leaving `password` out of a `[user]` table).
    pub user_password: Option<Option<String>>,
    pub ssh_keys: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataCard {
    Auto,
    Tf1,
    Tf2,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wifi {
    pub ssid: String,
    /// `None`: an open network.
    pub password: Option<String>,
}

/// Why a file was refused, in words that never repeat a value from it.
pub type Refusal = String;

fn table<'a>(v: &'a Value, what: &str) -> Result<&'a toml::Table, Refusal> {
    v.as_table()
        .ok_or(format!("{what} must be a table ([{what}])"))
}

fn text(v: &Value, what: &str, min: usize, max: usize) -> Result<String, Refusal> {
    let s = v.as_str().ok_or(format!("{what} must be text in quotes"))?;
    if s.chars().any(|c| c == '\n' || c == '\r' || c == '\0') {
        return Err(format!("{what} has a line break or a NUL in it"));
    }
    if s.len() < min || s.len() > max {
        return Err(format!("{what} must be {min} to {max} bytes long"));
    }
    Ok(s.to_owned())
}

fn only(t: &toml::Table, what: &str, allowed: &[&str]) -> Result<(), Refusal> {
    match t.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(k) => Err(format!("{what} has an unknown setting: {k}")),
        None => Ok(()),
    }
}

/// A time zone name: a path under /usr/share/zoneinfo, with no way out of it.
fn valid_timezone(z: &str) -> bool {
    !z.is_empty()
        && z.len() <= 64
        && !z.starts_with('/')
        && z.split('/').all(|p| !p.is_empty() && p != "." && p != "..")
        && z.chars()
            .all(|c| c.is_ascii_alphanumeric() || "/_+-".contains(c))
}

/// One authorized_keys line: a known key type and base64 key, an optional comment.
fn valid_ssh_key(k: &str) -> bool {
    const TYPES: &[&str] = &[
        "ssh-ed25519",
        "ssh-rsa",
        "ecdsa-sha2-nistp256",
        "ecdsa-sha2-nistp384",
        "ecdsa-sha2-nistp521",
        "sk-ssh-ed25519@openssh.com",
        "sk-ecdsa-sha2-nistp256@openssh.com",
    ];
    let mut parts = k.split_whitespace();
    let (Some(kind), Some(key)) = (parts.next(), parts.next()) else {
        return false;
    };
    TYPES.contains(&kind)
        && key.len() >= 16
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
}

/// Checks a file's text; an editor's byte-order mark and Windows line ends are fine.
pub fn parse(text_in: &str) -> Result<Provision, Refusal> {
    let text_in = text_in.strip_prefix('\u{feff}').unwrap_or(text_in);
    let doc: toml::Table = text_in
        .parse()
        // toml's message can quote the line, and with it a password: give only where.
        .map_err(|e: toml::de::Error| match e.span() {
            Some(span) => format!(
                "not valid TOML (line {})",
                text_in[..span.start].matches('\n').count() + 1
            ),
            None => "not valid TOML".into(),
        })?;
    only(
        &doc,
        "the file",
        &["timezone", "data_card", "wifi", "user", "ssh"],
    )?;
    let mut p = Provision::default();
    if let Some(v) = doc.get("timezone") {
        let z = text(v, "timezone", 1, 64)?;
        if !valid_timezone(&z) {
            return Err("timezone isn't a time zone name like Europe/London".into());
        }
        p.timezone = Some(z);
    }
    if let Some(v) = doc.get("data_card") {
        p.data_card = Some(match text(v, "data_card", 1, 8)?.as_str() {
            "auto" => DataCard::Auto,
            "tf1" => DataCard::Tf1,
            "tf2" => DataCard::Tf2,
            _ => return Err("data_card must be auto, tf1 or tf2".into()),
        });
    }
    if let Some(v) = doc.get("wifi") {
        let t = table(v, "wifi")?;
        only(t, "[wifi]", &["ssid", "password"])?;
        let ssid = text(
            t.get("ssid").ok_or("[wifi] needs an ssid")?,
            "[wifi] ssid",
            1,
            32,
        )?;
        let password = match t.get("password") {
            Some(v) => Some(text(v, "[wifi] password", 8, 63)?),
            None => None,
        };
        p.wifi = Some(Wifi { ssid, password });
    }
    if let Some(v) = doc.get("user") {
        let t = table(v, "user")?;
        only(t, "[user]", &["password"])?;
        p.user_password = Some(match t.get("password") {
            Some(v) => Some(text(v, "[user] password", 1, 256)?),
            None => None,
        });
    }
    if let Some(v) = doc.get("ssh") {
        let t = table(v, "ssh")?;
        only(t, "[ssh]", &["authorized_keys"])?;
        let list = t
            .get("authorized_keys")
            .ok_or("[ssh] needs authorized_keys")?
            .as_array()
            .ok_or("[ssh] authorized_keys must be a list: [\"ssh-ed25519 ...\"]")?;
        let mut keys = Vec::new();
        for (i, k) in list.iter().enumerate() {
            let k = text(k, &format!("[ssh] key {}", i + 1), 1, 16384)?;
            if !valid_ssh_key(&k) {
                return Err(format!(
                    "[ssh] key {} isn't a public key line (ssh-ed25519 AAAA... comment)",
                    i + 1
                ));
            }
            keys.push(k);
        }
        p.ssh_keys = Some(keys);
    }
    Ok(p)
}

/// iwd's file name for a network: the name itself when it is only letters, digits, - and _,
/// otherwise `=` and the name in hex; `.psk` with a passphrase, `.open` without.
pub fn iwd_file_name(w: &Wifi) -> String {
    let plain = w
        .ssid
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    let stem = if plain {
        w.ssid.clone()
    } else {
        let hex: String = w.ssid.bytes().map(|b| format!("{b:02x}")).collect();
        format!("={hex}")
    };
    format!(
        "{stem}.{}",
        if w.password.is_some() { "psk" } else { "open" }
    )
}

/// The iwd network file's contents.
pub fn iwd_file(w: &Wifi) -> String {
    match &w.password {
        Some(p) => format!("[Security]\nPassphrase={p}\n"),
        None => "[Settings]\nAutoConnect=true\n".into(),
    }
}

/// The name a refused file is kept under: the first free `provision.rejected-N.toml`.
pub fn rejected_name(existing: &BTreeMap<String, ()>) -> String {
    (1..)
        .map(|n| format!("provision.rejected-{n}.toml"))
        .find(|n| !existing.contains_key(n))
        .expect("an unused number")
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIN5ZmKEUPNxLeq8mi7VYyk9F1oj1bV0G9Ae/s+H7Osnx me@mac";

    #[test]
    fn a_full_file() {
        let p = parse(&format!(
            "\u{feff}timezone = \"Europe/London\"\r\ndata_card = \"tf2\"\r\n\
             [wifi]\r\nssid = \"My Net\"\r\npassword = \"correct horse\"\r\n\
             [user]\r\npassword = \"s3cret\"\r\n\
             [ssh]\r\nauthorized_keys = [\"{KEY}\"]\r\n"
        ))
        .unwrap();
        assert_eq!(p.timezone.as_deref(), Some("Europe/London"));
        assert_eq!(p.data_card, Some(DataCard::Tf2));
        let w = p.wifi.unwrap();
        assert_eq!(iwd_file_name(&w), "=4d79204e6574.psk");
        assert_eq!(iwd_file(&w), "[Security]\nPassphrase=correct horse\n");
        assert_eq!(p.user_password, Some(Some("s3cret".into())));
        assert_eq!(p.ssh_keys.unwrap(), [KEY]);
    }

    #[test]
    fn empty_and_partial_files() {
        assert_eq!(parse("").unwrap(), Provision::default());
        let p = parse("[user]\n").unwrap();
        assert_eq!(
            p.user_password,
            Some(None),
            "a [user] without a password: none"
        );
        let open = parse("[wifi]\nssid = \"cafe_1\"\n").unwrap().wifi.unwrap();
        assert_eq!(iwd_file_name(&open), "cafe_1.open");
    }

    #[test]
    fn refusals_never_quote_values() {
        let secret = "hunter2-not-in-errors";
        let cases = [
            format!("[wifi]\nssid = \"x\"\npassword = \"{secret}\"\nextra = 1\n"),
            format!("[wifi]\nssid = \"x\"\npassword = \"short\"\n# {secret}\n"),
            format!("[user]\npassword = \"a\\nb{secret}\"\n"),
            format!("[user]\nname = \"{secret}\"\n"),
            format!("[user]\npassword = \"{secret}\n"), // broken TOML
            format!("[ssh]\nauthorized_keys = [\"not a key {secret}\"]\n"),
            format!("timezone = \"../../etc/{secret}\"\n"),
            format!("data_card = \"tf3\"\nwhatever = \"{secret}\"\n"),
        ];
        for c in cases {
            let e = parse(&c).unwrap_err();
            assert!(!e.contains(secret), "{e}");
        }
    }

    #[test]
    fn specific_checks() {
        assert!(parse("[wifi]\nssid = \"\"\n").is_err());
        assert!(parse(&format!("[wifi]\nssid = \"{}\"\n", "x".repeat(33))).is_err());
        assert!(parse(&format!(
            "[wifi]\nssid = \"a\"\npassword = \"{}\"\n",
            "p".repeat(64)
        ))
        .is_err());
        assert!(parse("[ssh]\n").is_err(), "[ssh] without keys");
        assert!(parse("wifi = \"x\"\n").is_err(), "wifi not a table");
        assert!(parse("timezone = \"/etc/passwd\"\n").is_err());
        assert!(parse("timezone = \"America/Argentina/Buenos_Aires\"\n").is_ok());
        assert_eq!(
            parse("[user]\npassword = \"x\"\nnope = 1\n").unwrap_err(),
            "[user] has an unknown setting: nope"
        );
    }

    #[test]
    fn rejected_names() {
        let mut m = BTreeMap::new();
        assert_eq!(rejected_name(&m), "provision.rejected-1.toml");
        m.insert("provision.rejected-1.toml".into(), ());
        assert_eq!(rejected_name(&m), "provision.rejected-2.toml");
    }
}
