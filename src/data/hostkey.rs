//! Ilk baglantida host anahtari dogrulamasi.
//!
//! `SSH_ASKPASS_REQUIRE=force` verdigimiz icin OpenSSH "bu sunucuya
//! guveniyor musun?" sorusunu da askpass'e yonlendiriyor; bizim askpass
//! parola donduruyor, dolayisiyla soru kullaniciya hic ulasmiyordu.
//!
//! Cozum soruyu uygulamanin kendi diyalogu yapmak: anahtari `ssh-keyscan`
//! ile aliyoruz, parmak izini kullaniciya gosteriyoruz, onay verirse
//! `known_hosts`'a yaziyoruz. Otomatik kabul etmiyoruz — guven karari
//! kullanicinin.

use std::path::PathBuf;
use std::process::Stdio;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Known {
    /// known_hosts'ta kayitli; dogrudan baglanilabilir.
    Yes,
    /// Hic gorulmemis; onay gerekiyor.
    No,
    /// Karar verilemedi (ssh-keygen yok vb.) — ssh'in kendi akisina birak.
    Unclear,
}

pub fn known_hosts() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".ssh/known_hosts"))
}

/// `[host]:port` — ssh'in standart gosterimi.
fn lookup_name(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_string()
    } else {
        format!("[{host}]:{port}")
    }
}

pub fn status(host: &str, port: u16) -> Known {
    let Some(path) = known_hosts() else {
        return Known::Unclear;
    };

    if !path.exists() {
        return Known::No;
    }

    let output = std::process::Command::new("ssh-keygen")
        .arg("-F")
        .arg(lookup_name(host, port))
        .arg("-f")
        .arg(&path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();

    match output {
        Ok(output) if output.status.success() && !output.stdout.is_empty() => Known::Yes,
        Ok(_) => Known::No,
        Err(_) => Known::Unclear,
    }
}

/// `ssh-keyscan` ciktisi: her satir bir anahtar.
pub fn scan(host: &str, port: u16) -> Result<Vec<String>, String> {
    let output = std::process::Command::new("ssh-keyscan")
        .arg("-T")
        .arg("5")
        .arg("-p")
        .arg(port.to_string())
        .arg(host)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("ssh-keyscan calistirilamadi: {error}"))?;

    let keys: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect();

    if keys.is_empty() {
        let reason = String::from_utf8_lossy(&output.stderr)
            .lines()
            .next_back()
            .unwrap_or("sunucudan anahtar alinamadi")
            .trim()
            .to_string();
        return Err(reason);
    }

    Ok(keys)
}

/// Kullaniciya gosterilecek parmak izleri: ("ED25519", "SHA256:...").
pub fn fingerprints(keys: &[String]) -> Vec<(String, String)> {
    use std::io::Write;

    let mut child = match std::process::Command::new("ssh-keygen")
        .arg("-l")
        .arg("-f")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return Vec::new(),
    };

    if let Some(stdin) = child.stdin.as_mut() {
        let _ = stdin.write_all(keys.join("\n").as_bytes());
    }

    let Ok(output) = child.wait_with_output() else {
        return Vec::new();
    };

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            // "256 SHA256:xxx host (ED25519)"
            let mut fields = line.split_whitespace();
            let _bits = fields.next()?;
            let fingerprint = fields.next()?.to_string();
            let kind = line
                .rsplit('(')
                .next()?
                .trim_end_matches(')')
                .to_string();
            Some((kind, fingerprint))
        })
        .collect()
}

/// Anahtarlari known_hosts'a ekler. Dosyayi yeniden yazmaz, sadece ekler.
pub fn trust(keys: &[String]) -> Result<(), String> {
    let path = known_hosts().ok_or_else(|| "~/.ssh bulunamadi".to_string())?;
    trust_into(&path, keys)
}

fn trust_into(path: &std::path::Path, keys: &[String]) -> Result<(), String> {
    use std::io::Write;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("{} olusturulamadi: {error}", parent.display()))?;
    }

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("known_hosts acilamadi: {error}"))?;

    for key in keys {
        writeln!(file, "{key}")
            .map_err(|error| format!("known_hosts yazilamadi: {error}"))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_is_part_of_the_lookup_name() {
        assert_eq!(lookup_name("10.0.0.1", 22), "10.0.0.1");
        assert_eq!(lookup_name("10.0.0.1", 2222), "[10.0.0.1]:2222");
    }

    /// Var olan kayitlarin uzerine yazmamali: dosya sadece buyumeli.
    #[test]
    fn trust_appends_without_touching_existing_entries() {
        let dir = std::env::temp_dir().join(format!("runetty-test-{}", std::process::id()));
        let path = dir.join("known_hosts");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(&path, "old.example ssh-rsa AAAA\n").unwrap();

        trust_into(&path, &["new.example ssh-ed25519 BBBB".to_string()]).unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("old.example"), "eski kayit korunmali");
        assert!(contents.contains("new.example"), "yeni kayit eklenmeli");
        assert_eq!(contents.lines().count(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_fingerprints_from_keyscan_lines() {
        // ssh-keygen yoksa test anlamsiz; ortamda varsa gercek ciktiyi dogrula.
        if std::process::Command::new("ssh-keygen").arg("-?").output().is_err() {
            return;
        }

        let keys = vec![
            "example.test ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJ1IYbMBDJqUPNa0zRcVIcA1UGQCGQcYfVKPTFIbTkSX".to_string(),
        ];

        let found = fingerprints(&keys);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "ED25519");
        assert!(found[0].1.starts_with("SHA256:"), "got {}", found[0].1);
    }
}
