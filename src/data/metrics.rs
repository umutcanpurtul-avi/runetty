//! Collecting and parsing host metrics.
//!
//! Cost is the one real risk here: ten hosts every two seconds would mean a
//! fresh SSH handshake per round. Instead `ControlMaster` opens one persistent
//! channel per host and every later round reuses it, so the cost drops to
//! running a single command.

/// The command run remotely in one round. Section headers keep parsing
/// stable: we avoid tools whose output format shifts (top, vmstat).
pub const PROBE: &str = "\
printf 'STAT\\n'; head -1 /proc/stat; \
printf 'MEM\\n'; grep -E '^(MemTotal|MemAvailable):' /proc/meminfo; \
printf 'UP\\n'; cut -d' ' -f1 /proc/uptime; \
printf 'CPUS\\n'; nproc 2>/dev/null || grep -c ^processor /proc/cpuinfo; \
printf 'DISK\\n'; df -P / | tail -1";

/// CPU scheduler counters. The percentage comes from the delta between two samples.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuTimes {
    pub total: u64,
    pub idle: u64,
    pub user: u64,
    pub system: u64,
    pub iowait: u64,
    pub steal: u64,
}

/// Where the CPU time went between two samples, as percentages.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CpuBreakdown {
    pub user: f64,
    pub system: f64,
    pub iowait: f64,
    pub steal: f64,
    pub idle: f64,
}

impl CpuBreakdown {
    pub fn busy(&self) -> f64 {
        (100.0 - self.idle).clamp(0.0, 100.0)
    }
}

/// Needs two samples, like `cpu_percent`.
pub fn cpu_breakdown(previous: Option<CpuTimes>, current: CpuTimes) -> Option<CpuBreakdown> {
    let previous = previous?;
    let total = current.total.checked_sub(previous.total)?;
    if total == 0 {
        return None;
    }

    let share = |now: u64, before: u64| {
        now.checked_sub(before).unwrap_or(0) as f64 * 100.0 / total as f64
    };

    Some(CpuBreakdown {
        user: share(current.user, previous.user),
        system: share(current.system, previous.system),
        iowait: share(current.iowait, previous.iowait),
        steal: share(current.steal, previous.steal),
        idle: share(current.idle, previous.idle),
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub cpu: CpuTimes,
    pub mem_total_kb: u64,
    pub mem_available_kb: u64,
    pub mem_free_kb: u64,
    pub mem_cached_kb: u64,
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

    /// Summary line at the foot of a card: "up 5d 13:52 · 4c · 5.8 GB"
    pub fn summary(&self) -> String {
        format!(
            "{} · {}c · {:.1} GB",
            self.uptime_text(),
            self.cores,
            self.memory_total_gb()
        )
    }
}

/// CPU usage between two samples. The first round has no previous sample, so
/// it returns `None`; showing zero would be misleading.
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
    let mut mem_free = 0;
    let mut mem_cached = 0;
    let mut uptime = 0;
    let mut cores = 0;
    let mut disk_used = 0.0;
    let mut disk_total = 0;

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // All section names are recognised in one place. When the extra
        // detail sections (LOAD, NET, PROCS) were not, they counted as part of
        // the previous section and zeroed out things like the core count.
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
                } else if line.starts_with("MemFree") {
                    mem_free = value;
                } else if line.starts_with("Cached") || line.starts_with("Buffers") {
                    // Page cache: Cached plus Buffers, as `free` reports it.
                    mem_cached += value;
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
        mem_free_kb: mem_free,
        mem_cached_kb: mem_cached,
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

    let at = |index: usize| values.get(index).copied().unwrap_or(0);

    // /proc/stat: user nice system idle iowait irq softirq steal
    // idle + iowait both mean "no work being done".
    Some(CpuTimes {
        total: values.iter().sum(),
        idle: values[3] + at(4),
        user: values[0] + at(1),
        system: values[2] + at(5) + at(6),
        iowait: at(4),
        steal: at(7),
    })
}

/// `ssh` arguments that run the probe over the persistent channel.
///
/// Same `ControlPath` as terminal sessions: with a tab open the probe rides
/// that channel and authenticates not at all. Without one, the password comes
/// from the keyring via SSH_ASKPASS - `BatchMode` is deliberately off, since
/// with it even a saved password could not be used.
pub fn probe_args(host: &super::Host) -> Vec<String> {
    host.ssh_invocation(BACKGROUND_OPTIONS, &[PROBE])
}

/// Options for probes that run without anyone watching. An unknown host is
/// never accepted silently - trust is decided in our own dialog - and a single
/// password attempt keeps a wrong answer from locking an account.
const BACKGROUND_OPTIONS: &[&str] = &[
    "-o",
    "StrictHostKeyChecking=yes",
    "-o",
    "ConnectTimeout=5",
    "-o",
    "NumberOfPasswordPrompts=1",
];

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
        let sample = parse(OUTPUT).expect("should parse");

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

    /// No previous sample on the first round: no value rather than zero.
    #[test]
    fn cpu_needs_two_samples() {
        let current = CpuTimes { total: 1000, idle: 850, ..CpuTimes::default() };
        assert_eq!(cpu_percent(None, current), None);

        let previous = CpuTimes { total: 900, idle: 800, ..CpuTimes::default() };
        let percent = cpu_percent(Some(previous), current).unwrap();
        assert!((percent - 50.0).abs() < 0.01, "got {percent}");
    }

    /// The breakdown must add up and reflect the per-field deltas.
    #[test]
    fn breakdown_splits_the_delta() {
        let previous = CpuTimes {
            total: 1000, idle: 800, user: 100, system: 50, iowait: 20, steal: 10,
        };
        let current = CpuTimes {
            total: 2000, idle: 1400, user: 300, system: 150, iowait: 60, steal: 90,
        };

        let breakdown = cpu_breakdown(Some(previous), current).unwrap();

        assert!((breakdown.user - 20.0).abs() < 0.01, "user {}", breakdown.user);
        assert!((breakdown.system - 10.0).abs() < 0.01);
        assert!((breakdown.steal - 8.0).abs() < 0.01);
        assert!((breakdown.idle - 60.0).abs() < 0.01);
        assert!((breakdown.busy() - 40.0).abs() < 0.01);
    }

    /// Rates need two readings and a time gap; a counter reset must read zero.
    #[test]
    fn rates_come_from_two_readings() {
        let before = Interface { name: "eth0".into(), rx_bytes: 1_000, tx_bytes: 2_000 };
        let after = Interface { name: "eth0".into(), rx_bytes: 3_048, tx_bytes: 2_000 };

        let measured = rate(&before, &after, 2.0);
        assert!((measured.rx - 1024.0).abs() < 0.01);
        assert_eq!(measured.tx, 0.0);
        assert_eq!(human_rate(measured.rx), "1 kB/s");

        let reset = Interface { name: "eth0".into(), rx_bytes: 10, tx_bytes: 10 };
        assert_eq!(rate(&after, &reset, 2.0).rx, 0.0);
    }

    /// Counters reset when a host reboots; the delta must not go negative.
    #[test]
    fn counter_reset_is_not_a_negative_percentage() {
        let previous = CpuTimes { total: 5000, idle: 4000, ..CpuTimes::default() };
        let current = CpuTimes { total: 100, idle: 90, ..CpuTimes::default() };
        assert_eq!(cpu_percent(Some(previous), current), None);
    }
}

/// The detail probe. It adds disk list, network counters and the busiest
/// processes on top of the card probe; still one command, still one channel.
pub const DETAIL_PROBE: &str = "\
printf 'STAT\\n'; head -1 /proc/stat; \
printf 'MEM\\n'; grep -E '^(MemTotal|MemAvailable|MemFree|Cached|Buffers|SwapTotal|SwapFree):' /proc/meminfo; \
printf 'UP\\n'; cut -d' ' -f1 /proc/uptime; \
printf 'CPUS\\n'; nproc 2>/dev/null || grep -c ^processor /proc/cpuinfo; \
printf 'LOAD\\n'; cut -d' ' -f1-3 /proc/loadavg; \
printf 'DISKS\\n'; df -P -x tmpfs -x devtmpfs -x squashfs -x efivarfs -x overlay 2>/dev/null | tail -n +2; \
printf 'NET\\n'; tail -n +3 /proc/net/dev; \
printf 'PROCS\\n'; ps -eo pcpu,pmem,pid,args --sort=-pcpu 2>/dev/null | tail -n +2 | head -12";

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

/// Per-second rates, derived from two readings of the same counters.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rate {
    pub rx: f64,
    pub tx: f64,
}

/// Counters reset on reboot or when an interface is recreated; a negative
/// delta means "no rate" rather than a wild number.
pub fn rate(previous: &Interface, current: &Interface, seconds: f64) -> Rate {
    if seconds <= 0.0 {
        return Rate::default();
    }

    let delta = |now: u64, before: u64| {
        now.checked_sub(before).map(|d| d as f64 / seconds).unwrap_or(0.0)
    };

    Rate {
        rx: delta(current.rx_bytes, previous.rx_bytes),
        tx: delta(current.tx_bytes, previous.tx_bytes),
    }
}

/// "144 kB/s"
pub fn human_rate(bytes_per_second: f64) -> String {
    let value = bytes_per_second.max(0.0);
    if value >= 1_048_576.0 {
        format!("{:.1} MB/s", value / 1_048_576.0)
    } else if value >= 1024.0 {
        format!("{:.0} kB/s", value / 1024.0)
    } else {
        format!("{value:.0} B/s")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Process {
    pub cpu: f64,
    pub mem: f64,
    pub pid: u32,
    pub name: String,
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
                // Bind mounts show the same filesystem many times over;
                // the first mount point seen is enough.
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
        // The card probe uses the singular header, the detail probe the plural.
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
        // A mount point may contain spaces; rejoin the remaining fields.
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
    // /proc/net/dev: eight rx columns, then eight tx columns.
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
    host.ssh_invocation(BACKGROUND_OPTIONS, &[DETAIL_PROBE])
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

    /// LOAD, NET and PROCS used to be unrecognised and counted as part of the
    /// previous section, which zeroed out the core count.
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
        assert_eq!(detail.disks.len(), 2, "a bind mount must not be listed twice");
    }

    #[test]
    fn parses_detail_sections() {
        let detail = parse_detail(OUTPUT).expect("should parse");

        assert_eq!(detail.load, [0.52, 0.71, 0.93]);
        assert!((detail.swap_percent() - 25.0).abs() < 0.01);

        assert_eq!(detail.disks.len(), 2);
        assert_eq!(detail.disks[0].mount, "/");
        assert_eq!(detail.disks[1].used_percent, 2.0);

        assert_eq!(detail.interfaces.len(), 1, "lo must not be listed");
        assert_eq!(detail.interfaces[0].name, "eth0");
        assert_eq!(detail.interfaces[0].rx_bytes, 1_024_000);
        assert_eq!(detail.interfaces[0].tx_bytes, 2_048_000);

        assert_eq!(detail.processes.len(), 2);
        assert_eq!(detail.processes[0].name, "postgres");
        assert_eq!(detail.processes[0].pid, 1234);
    }

    /// A mount point may contain spaces; column rejoining must hold.
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
