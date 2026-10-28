//! Sunucu metriklerinin toplanmasi ve ayristirilmasi.
//!
//! Maliyet buradaki tek gercek risk: 10 sunucu x 2 saniye, her turda yeni bir
//! SSH el sikismasi demek olurdu. Bunun yerine `ControlMaster` ile sunucu
//! basina tek kalici kanal aciliyor; sonraki her turda ayni kanal kullaniliyor,
//! yani maliyet bir komut calistirmaya iniyor.

use std::path::PathBuf;

/// Tek turda uzak tarafta calisan komut. Bolum basliklari ayristirmayi
/// sabit tutuyor: cikti bicimi degisen araclara (top, vmstat) guvenmiyoruz.
pub const PROBE: &str = "\
printf 'STAT\\n'; head -1 /proc/stat; \
printf 'MEM\\n'; grep -E '^(MemTotal|MemAvailable):' /proc/meminfo; \
printf 'UP\\n'; cut -d' ' -f1 /proc/uptime; \
printf 'CPUS\\n'; nproc 2>/dev/null || grep -c ^processor /proc/cpuinfo; \
printf 'DISK\\n'; df -P / | tail -1";

/// CPU zamanlayici sayaclari. Yuzde, iki olcum arasindaki farktan cikiyor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuTimes {
    pub total: u64,
    pub idle: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub cpu: CpuTimes,
    pub mem_total_kb: u64,
    pub mem_available_kb: u64,
    pub uptime_seconds: u64,
    pub cores: u32,
    pub disk_used_percent: f64,
    pub disk_total_kb: u64,
}

impl Sample {
    pub fn memory_percent(&self) -> f64 {
        if self.mem_total_kb == 0 {
            return 0.0;
        }
        let used = self.mem_total_kb.saturating_sub(self.mem_available_kb);
        used as f64 * 100.0 / self.mem_total_kb as f64
    }

    pub fn memory_total_gb(&self) -> f64 {
        self.mem_total_kb as f64 / 1_048_576.0
    }

    /// "up 5d 13:52"
    pub fn uptime_text(&self) -> String {
        let days = self.uptime_seconds / 86_400;
        let hours = (self.uptime_seconds % 86_400) / 3_600;
        let minutes = (self.uptime_seconds % 3_600) / 60;

        if days > 0 {
            format!("up {days}d {hours:02}:{minutes:02}")
        } else {
            format!("up {hours:02}:{minutes:02}")
        }
    }

    /// Kart altligindaki ozet satiri: "up 5d 13:52 · 4c · 5.8 GB"
    pub fn summary(&self) -> String {
        format!(
            "{} · {}c · {:.1} GB",
            self.uptime_text(),
            self.cores,
            self.memory_total_gb()
        )
    }
}

/// Iki olcum arasindaki CPU kullanimi. Ilk turda onceki olcum olmadigi icin
/// `None` doner — sifir gostermek yaniltici olurdu.
pub fn cpu_percent(previous: Option<CpuTimes>, current: CpuTimes) -> Option<f64> {
    let previous = previous?;

    let total = current.total.checked_sub(previous.total)?;
    let idle = current.idle.checked_sub(previous.idle)?;
    if total == 0 {
        return None;
    }

    let busy = total.saturating_sub(idle);
    Some((busy as f64 * 100.0 / total as f64).clamp(0.0, 100.0))
}

pub fn parse(output: &str) -> Option<Sample> {
    let mut section = "";
    let mut cpu = None;
    let mut mem_total = 0;
    let mut mem_available = 0;
    let mut uptime = 0;
    let mut cores = 0;
    let mut disk_used = 0.0;
    let mut disk_total = 0;

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        match line {
            "STAT" | "MEM" | "UP" | "CPUS" | "DISK" => {
                section = match line {
                    "STAT" => "stat",
                    "MEM" => "mem",
                    "UP" => "up",
                    "CPUS" => "cpus",
                    _ => "disk",
                };
                continue;
            }
            _ => {}
        }

        match section {
            "stat" => cpu = parse_cpu(line),
            "mem" => {
                let value = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or(0);

                if line.starts_with("MemTotal") {
                    mem_total = value;
                } else if line.starts_with("MemAvailable") {
                    mem_available = value;
                }
            }
            "up" => uptime = line.split('.').next()?.parse().ok()?,
            "cpus" => cores = line.parse().unwrap_or(0),
            "disk" => {
                let fields: Vec<&str> = line.split_whitespace().collect();
                // df -P: Filesystem 1024-blocks Used Available Capacity Mounted
                if fields.len() >= 5 {
                    disk_total = fields[1].parse().unwrap_or(0);
                    disk_used = fields[4].trim_end_matches('%').parse().unwrap_or(0.0);
                }
            }
            _ => {}
        }
    }

    Some(Sample {
        cpu: cpu?,
        mem_total_kb: mem_total,
        mem_available_kb: mem_available,
        uptime_seconds: uptime,
        cores,
        disk_used_percent: disk_used,
        disk_total_kb: disk_total,
    })
}

fn parse_cpu(line: &str) -> Option<CpuTimes> {
    let values: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|value| value.parse().ok())
        .collect();

    if values.len() < 4 {
        return None;
    }

    // idle + iowait: ikisi de "is yapilmiyor" demek.
    let idle = values[3] + values.get(4).copied().unwrap_or(0);
    Some(CpuTimes { total: values.iter().sum(), idle })
}

/// ControlMaster soketleri. Kisa tutulmali: ssh soket yolu uzunlugu sinirli.
pub fn control_path() -> Option<PathBuf> {
    let base = dirs::runtime_dir().or_else(dirs::cache_dir)?;
    Some(base.join("runetty"))
}

/// Olcum komutunu kalici kanal uzerinden calistiran `ssh` argumanlari.
pub fn probe_args(target: &str) -> Vec<String> {
    let mut args: Vec<String> = vec!["ssh".into()];

    if let Some(dir) = control_path() {
        let _ = std::fs::create_dir_all(&dir);
        args.push("-o".into());
        args.push(format!("ControlPath={}/cm-%C", dir.display()));
        args.push("-o".into());
        args.push("ControlMaster=auto".into());
        args.push("-o".into());
        args.push("ControlPersist=120".into());
    }

    // Monitor arka planda calisiyor: parola soramayiz, takilmak yerine dusmeli.
    args.push("-o".into());
    args.push("BatchMode=yes".into());
    args.push("-o".into());
    args.push("ConnectTimeout=5".into());
    args.push(target.into());
    args.push(PROBE.into());

    args
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: &str = "\
STAT
cpu  100 0 50 800 50 0 0 0 0 0
MEM
MemTotal:        8000000 kB
MemAvailable:    2000000 kB
UP
470000.42
CPUS
4
DISK
/dev/sda1 100000000 41000000 59000000 41% /
";

    #[test]
    fn parses_probe_output() {
        let sample = parse(OUTPUT).expect("ayristirilabilmeli");

        assert_eq!(sample.cpu.total, 1000);
        assert_eq!(sample.cpu.idle, 850);
        assert_eq!(sample.cores, 4);
        assert_eq!(sample.disk_used_percent, 41.0);
        assert_eq!(sample.uptime_seconds, 470_000);
        assert!((sample.memory_percent() - 75.0).abs() < 0.01);
        assert!((sample.memory_total_gb() - 7.629).abs() < 0.01);
    }

    #[test]
    fn summary_matches_design() {
        let sample = parse(OUTPUT).unwrap();
        assert_eq!(sample.summary(), "up 5d 10:33 · 4c · 7.6 GB");
    }

    /// Ilk turda onceki olcum yok: sifir gostermek yerine deger yok.
    #[test]
    fn cpu_needs_two_samples() {
        let current = CpuTimes { total: 1000, idle: 850 };
        assert_eq!(cpu_percent(None, current), None);

        let previous = CpuTimes { total: 900, idle: 800 };
        let percent = cpu_percent(Some(previous), current).unwrap();
        assert!((percent - 50.0).abs() < 0.01, "got {percent}");
    }

    /// Sunucu yeniden baslayinca sayaclar sifirlanir; eksi fark cikmamali.
    #[test]
    fn counter_reset_is_not_a_negative_percentage() {
        let previous = CpuTimes { total: 5000, idle: 4000 };
        let current = CpuTimes { total: 100, idle: 90 };
        assert_eq!(cpu_percent(Some(previous), current), None);
    }
}
