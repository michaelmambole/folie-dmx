use std::io::Write;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use folie_dmx::enttec::{self, UsbPro};
use folie_dmx::{DmxOutput, NullOutput};
use folie_engine::{Command, EngineConfig, OutputBinding};

#[derive(Parser)]
#[command(name = "folie", version, about = "Folie-DMX : outils de diagnostic")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Liste les ports série (interfaces FTDI en premier).
    Ports,
    /// Interroge une interface Enttec DMX USB Pro.
    Info {
        #[arg(long)]
        port: String,
    },
    /// Règle le break et le MAB de l'USB Pro, en µs (gardés par l'interface, même débranchée).
    Reglages {
        #[arg(long)]
        port: String,
        #[arg(long)]
        break_us: f64,
        #[arg(long)]
        mab_us: f64,
    },
    /// Fait « respirer » des canaux. Sans --port, la sortie est virtuelle.
    Test {
        #[arg(long)]
        port: Option<String>,
        /// Canaux à animer : « 1 », « 1-4 », « 1,3,5-8 ».
        #[arg(long, default_value = "1-4")]
        channels: String,
        #[arg(long, default_value_t = 10.0)]
        seconds: f64,
    },
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    match Cli::parse().command {
        Cmd::Ports => ports(),
        Cmd::Info { port } => info(&port),
        Cmd::Reglages { port, break_us, mab_us } => settings(&port, break_us, mab_us),
        Cmd::Test { port, channels, seconds } => test(port.as_deref(), &channels, seconds),
    }
}

fn ports() -> Result<()> {
    let ports = enttec::find_ports()?;
    if ports.is_empty() {
        println!("Aucun port série détecté.");
    }
    for p in ports {
        let tag = if p.is_ftdi { "[FTDI]" } else { "      " };
        let desc = p.description.unwrap_or_default();
        let serial = p.serial_number.map(|s| format!(" (n° {s})")).unwrap_or_default();
        println!("{tag} {}  {desc}{serial}", p.path);
    }
    Ok(())
}

fn info(port: &str) -> Result<()> {
    let mut pro = UsbPro::open(port).with_context(|| format!("ouverture de {port}"))?;
    let params = pro.widget_parameters().context("lecture des paramètres")?;
    let serial = pro.serial_number().context("lecture du numéro de série")?;
    println!("Interface      : {}", pro.name());
    println!("Numéro de série: {serial:08}");
    println!(
        "Firmware       : {}.{}",
        params.firmware_version >> 8,
        params.firmware_version & 0xFF
    );
    println!("Break          : {:.0} µs", params.break_micros());
    println!("MAB            : {:.0} µs", params.mab_micros());
    let rate = match params.refresh_rate {
        0 => "maximum".to_string(),
        r => format!("{r} trames/s"),
    };
    println!("Rafraîchissement: {rate}");
    Ok(())
}

fn settings(port: &str, break_us: f64, mab_us: f64) -> Result<()> {
    let mut pro = UsbPro::open(port).with_context(|| format!("ouverture de {port}"))?;
    let rate = pro.widget_parameters().context("lecture des paramètres")?.refresh_rate;
    let units = |us: f64| (us / 10.67).round().clamp(0.0, 255.0) as u8;
    pro.set_widget_parameters(units(break_us), units(mab_us), rate).context("réglage")?;
    std::thread::sleep(Duration::from_millis(100));
    drop(pro);
    println!("Réglé. Relecture :");
    info(port)
}

fn test(port: Option<&str>, channels: &str, seconds: f64) -> Result<()> {
    let channels = parse_channels(channels)?;
    let output: Box<dyn DmxOutput> = match port {
        Some(p) => Box::new(UsbPro::open(p).with_context(|| format!("ouverture de {p}"))?),
        None => Box::new(NullOutput),
    };
    println!("Sortie : {} — canaux {channels:?} — Ctrl+C pour arrêter", output.name());

    let engine = folie_engine::spawn(
        EngineConfig::default(),
        vec![OutputBinding { universe: 0, output }],
    );

    let start = Instant::now();
    while start.elapsed().as_secs_f64() < seconds {
        let t = start.elapsed().as_secs_f64();
        for (i, &ch) in channels.iter().enumerate() {
            // Une sinusoïde par canal, décalée pour voir l'enchaînement.
            let phase = t * std::f64::consts::TAU / 2.0 - i as f64 * 0.6;
            let value = ((phase.sin() * 0.5 + 0.5) * 255.0).round() as u8;
            engine.set_channel(0, ch, value);
        }

        let snap = engine.snapshot();
        let bars: Vec<String> = channels
            .iter()
            .map(|&ch| {
                let v = snap.output[0].get(ch).unwrap_or(0);
                format!("{ch:>3}:{v:>3} {:<8}", "█".repeat(v as usize / 32))
            })
            .collect();
        print!(
            "\rtrame {:>6}  {}  erreurs {}  retards {}   ",
            snap.frame,
            bars.join(" "),
            snap.output_errors,
            snap.late_frames
        );
        std::io::stdout().flush()?;
        std::thread::sleep(Duration::from_millis(20));
    }

    engine.send(Command::Blackout(true));
    engine.shutdown();
    println!("\nTerminé, sortie mise au noir.");
    Ok(())
}

fn parse_channels(spec: &str) -> Result<Vec<u16>> {
    let mut out = Vec::new();
    for part in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => (a.trim().parse::<u16>()?, b.trim().parse::<u16>()?),
            None => {
                let n = part.parse::<u16>()?;
                (n, n)
            }
        };
        if a == 0 || b > 512 || a > b {
            bail!("plage de canaux invalide : {part}");
        }
        out.extend(a..=b);
    }
    if out.is_empty() {
        bail!("aucun canal donné");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::parse_channels;

    #[test]
    fn channel_specs() {
        assert_eq!(parse_channels("1-3,7").unwrap(), vec![1, 2, 3, 7]);
        assert!(parse_channels("0").is_err());
        assert!(parse_channels("5-2").is_err());
        assert!(parse_channels("513").is_err());
    }
}
