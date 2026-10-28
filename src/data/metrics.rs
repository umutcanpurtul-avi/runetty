//! Sunucu metriklerinin toplanmasi ve ayristirilmasi.
//!
//! Maliyet buradaki tek gercek risk: 10 sunucu x 2 saniye, her turda yeni bir
//! SSH el sikismasi demek olurdu. Bunun yerine `ControlMaster` ile sunucu
//! basina tek kalici kanal aciliyor; sonraki her turda ayni kanal kullaniliyor,
//! yani maliyet bir komut calistirmaya iniyor.

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

        // Butun bolum adlari tek yerden taniniyor. Detay ciktisindaki
        // ek bolumler (LOAD, NET, PROCS) burada taninmayinca onceki bolumun
        // icinde sayiliyordu ve ornegin cekirdek sayisi sifirlaniyordu.
        if let Some(name) = section_name(line) {
            section = name;
            continue;
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
            "disk" | "disks" => {
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

/// Olcum komutunu kalici kanal uzerinden calistiran `ssh` argumanlari.
///
/// Terminal oturumuyla ayni `ControlPath`: bir sekme aciksa olcum o kanaldan
/// gecer ve hicbir kimlik dogrulama yapilmaz. Kanal yoksa parola keyring'den
/// SSH_ASKPASS ile verilir — `BatchMode` bilerek kapali, cunku acikken
/// kaydedilmis parola da kullanilamiyordu.
pub fn probe_args(host: &super::Host) -> Vec<String> {
    let mut args = host.ssh_args();
    let target = args.pop().unwrap_or_default();

    // Bilinmeyen sunucuyu arka planda sessizce kabul etmek istemiyoruz;
    // guven karari kendi diyalogumuzda veriliyor.
    args.push("-o".into());
    args.push("StrictHostKeyChecking=yes".into());
    args.push("-o".into());
    args.push("ConnectTimeout=5".into());
    args.push("-o".into());
    args.push("NumberOfPasswordPrompts=1".into());

    args.push(target);
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

/// Detay sayfasinin olcumu. Kart olcumunun ustune disk listesi, ag sayaclari
/// ve en yuklu surecleri ekliyor; yine tek komut, yine ayni kalici kanal.
pub const DETAIL_PROBE: &str = "\
printf 'STAT\\n'; head -1 /proc/stat; \
printf 'MEM\\n'; grep -E '^(MemTotal|MemAvailable|SwapTotal|SwapFree):' /proc/meminfo; \
printf 'UP\\n'; cut -d' ' -f1 /proc/uptime; \
printf 'CPUS\\n'; nproc 2>/dev/null || grep -c ^processor /proc/cpuinfo; \
printf 'LOAD\\n'; cut -d' ' -f1-3 /proc/loadavg; \
printf 'DISKS\\n'; df -P -x tmpfs -x devtmpfs -x squashfs -x efivarfs -x overlay 2>/dev/null | tail -n +2; \
printf 'NET\\n'; tail -n +3 /proc/net/dev; \
printf 'PROCS\\n'; ps -eo pcpu,pmem,pid,comm --sort=-pcpu 2>/dev/null | tail -n +2 | head -10";

#[derive(Clone, Debug, PartialEq)]
pub struct Disk {
    pub device: String,
    pub mount: String,
    pub total_kb: u64,
    pub used_kb: u64,
    pub used_percent: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Interface {
    pub name: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Process {
    pub cpu: f64,
    pub mem: f64,
    pub pid: u32,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Detail {
    pub sample: Sample,
    pub swap_total_kb: u64,
    pub swap_free_kb: u64,
    pub load: [f64; 3],
    pub disks: Vec<Disk>,
    pub interfaces: Vec<Interface>,
    pub processes: Vec<Process>,
}

impl Detail {
    pub fn swap_percent(&self) -> f64 {
        if self.swap_total_kb == 0 {
            return 0.0;
        }
        let used = self.swap_total_kb.saturating_sub(self.swap_free_kb);
        used as f64 * 100.0 / self.swap_total_kb as f64
    }
}

pub fn parse_detail(output: &str) -> Option<Detail> {
    let sample = parse(output)?;

    let mut section = "";
    let mut swap_total = 0;
    let mut swap_free = 0;
    let mut load = [0.0; 3];
    let mut disks = Vec::new();
    let mut interfaces = Vec::new();
    let mut processes = Vec::new();

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(name) = section_name(line) {
            section = name;
            continue;
        }

        match section {
            "mem" => {
                let value = field(line, 1).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
                if line.starts_with("SwapTotal") {
                    swap_total = value;
                } else if line.starts_with("SwapFree") {
                    swap_free = value;
                }
            }
            "load" => {
                for (index, value) in line.split_whitespace().take(3).enumerate() {
                    load[index] = value.parse().unwrap_or(0.0);
                }
            }
            "disks" => {
                // Bind mount'lar ayni dosya sistemini defalarca gosteriyor;
                // ilk goruldugu baglanti noktasi yeterli.
                if let Some(disk) = parse_disk(line) {
                    if !disks.iter().any(|seen: &Disk| seen.device == disk.device) {
                        disks.push(disk);
                    }
                }
            }
            "net" => {
                if let Some(interface) = parse_interface(line) {
                    interfaces.push(interface);
                }
            }
            "procs" => {
                if let Some(process) = parse_process(line) {
                    processes.push(process);
                }
            }
            _ => {}
        }
    }

    Some(Detail {
        sample,
        swap_total_kb: swap_total,
        swap_free_kb: swap_free,
        load,
        disks,
        interfaces,
        processes,
    })
}

fn section_name(line: &str) -> Option<&'static str> {
    match line {
        "STAT" => Some("stat"),
        "MEM" => Some("mem"),
        "UP" => Some("up"),
        "CPUS" => Some("cpus"),
        "LOAD" => Some("load"),
        // Kart olcumu tekil, detay olcumu cogul basligi kullaniyor.
        "DISK" => Some("disk"),
        "DISKS" => Some("disks"),
        "NET" => Some("net"),
        "PROCS" => Some("procs"),
        _ => None,
    }
}

fn field(line: &str, index: usize) -> Option<&str> {
    line.split_whitespace().nth(index)
}

fn parse_disk(line: &str) -> Option<Disk> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    if fields.len() < 6 {
        return None;
    }

    Some(Disk {
        device: fields[0].to_string(),
        total_kb: fields[1].parse().ok()?,
        used_kb: fields[2].parse().ok()?,
        used_percent: fields[4].trim_end_matches('%').parse().ok()?,
        // Bagli dizin bosluk icerebilir; kalan alanlari geri birlestir.
        mount: fields[5..].join(" "),
    })
}

fn parse_interface(line: &str) -> Option<Interface> {
    let (name, rest) = line.split_once(':')?;
    let name = name.trim();
    if name == "lo" {
        return None;
    }

    let values: Vec<&str> = rest.split_whitespace().collect();
    // /proc/net/dev: rx 8 sutun, sonra tx 8 sutun.
    Some(Interface {
        name: name.to_string(),
        rx_bytes: values.first()?.parse().ok()?,
        tx_bytes: values.get(8)?.parse().ok()?,
    })
}

fn parse_process(line: &str) -> Option<Process> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    if fields.len() < 4 {
        return None;
    }

    Some(Process {
        cpu: fields[0].parse().ok()?,
        mem: fields[1].parse().ok()?,
        pid: fields[2].parse().ok()?,
        name: fields[3..].join(" "),
    })
}

/// "5.8 GB", "812 MB"
pub fn human_kb(kilobytes: u64) -> String {
    const UNITS: [(&str, f64); 4] = [
        ("TB", 1_073_741_824.0),
        ("GB", 1_048_576.0),
        ("MB", 1_024.0),
        ("kB", 1.0),
    ];

    let value = kilobytes as f64;
    for (unit, scale) in UNITS {
        if value >= scale {
            let scaled = value / scale;
            return if scaled >= 100.0 {
                format!("{scaled:.0} {unit}")
            } else {
                format!("{scaled:.1} {unit}")
            };
        }
    }

    "0 kB".into()
}

pub fn detail_args(host: &super::Host) -> Vec<String> {
    let mut args = probe_args(host);
    args.pop();
    args.push(DETAIL_PROBE.into());
    args
}

#[cfg(test)]
mod detail_tests {
    use super::*;

    const OUTPUT: &str = "\
STAT
cpu  100 0 50 800 50 0 0 0 0 0
MEM
MemTotal:        8000000 kB
MemAvailable:    2000000 kB
SwapTotal:       2000000 kB
SwapFree:        1500000 kB
UP
470000.42
CPUS
4
LOAD
0.52 0.71 0.93
DISKS
/dev/sda1 100000000 41000000 59000000 41% /
/dev/sdb1 500000000 10000000 490000000 2% /mnt/data store
NET
    eth0: 1024000 100 0 0 0 0 0 0 2048000 200 0 0 0 0 0 0
      lo: 500 5 0 0 0 0 0 0 500 5 0 0 0 0 0 0
PROCS
 12.5  3.2  1234 postgres
  4.0  1.1  2345 sshd
";

    /// LOAD, NET, PROCS bolumleri eskiden taninmiyor ve bir onceki bolumun
    /// icinde sayiliyordu; cekirdek sayisi boylece sifirlaniyordu.
    #[test]
    fn extra_sections_do_not_clobber_earlier_ones() {
        let detail = parse_detail(OUTPUT).unwrap();
        assert_eq!(detail.sample.cores, 4);
    }

    #[test]
    fn same_filesystem_is_listed_once() {
        let text = OUTPUT.replace(
            "/dev/sdb1 500000000 10000000 490000000 2% /mnt/data store",
            "/dev/sdb1 500000000 10000000 490000000 2% /mnt/data store\n\
             /dev/sda1 100000000 41000000 59000000 41% /home",
        );

        let detail = parse_detail(&text).unwrap();
        assert_eq!(detail.disks.len(), 2, "bind mount tekrar listelenmemeli");
    }

    #[test]
    fn parses_detail_sections() {
        let detail = parse_detail(OUTPUT).expect("ayristirilabilmeli");

        assert_eq!(detail.load, [0.52, 0.71, 0.93]);
        assert!((detail.swap_percent() - 25.0).abs() < 0.01);

        assert_eq!(detail.disks.len(), 2);
        assert_eq!(detail.disks[0].mount, "/");
        assert_eq!(detail.disks[1].used_percent, 2.0);

        assert_eq!(detail.interfaces.len(), 1, "lo listelenmemeli");
        assert_eq!(detail.interfaces[0].name, "eth0");
        assert_eq!(detail.interfaces[0].rx_bytes, 1_024_000);
        assert_eq!(detail.interfaces[0].tx_bytes, 2_048_000);

        assert_eq!(detail.processes.len(), 2);
        assert_eq!(detail.processes[0].name, "postgres");
        assert_eq!(detail.processes[0].pid, 1234);
    }

    /// Bagli dizin bosluk icerebiliyor; sutun birlestirme bozulmamali.
    #[test]
    fn mount_point_with_space_is_kept_whole() {
        let detail = parse_detail(OUTPUT).unwrap();
        assert_eq!(detail.disks[1].mount, "/mnt/data store");
    }

    #[test]
    fn formats_sizes() {
        assert_eq!(human_kb(1_024), "1.0 MB");
        assert_eq!(human_kb(6_082_740), "5.8 GB");
        assert_eq!(human_kb(512), "512 kB");
    }
}
