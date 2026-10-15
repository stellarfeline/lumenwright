use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use lumenwright::assets::jar;
use lumenwright::job::{self, RenderArgs};
use lumenwright::world::{Dimension, World};

/// Offline headless renderer for Minecraft Java Edition worlds.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Log verbosity (error, warn, info, debug, trace).
    #[arg(long, global = true, default_value = "info")]
    log: String,
    /// Worker threads for CPU work (default: all cores).
    #[arg(long, global = true)]
    threads: Option<usize>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Render an image of a world.
    Render(Box<RenderArgs>),
    /// Print world metadata and statistics.
    Info {
        world: PathBuf,
        #[arg(long, default_value = "overworld")]
        dimension: String,
        /// Also count entities per region (slower).
        #[arg(long)]
        entities: bool,
    },
    /// Manage the vanilla client jar used for textures and models.
    Assets {
        #[command(subcommand)]
        cmd: AssetsCmd,
    },
    /// List the rendering backends available on this machine.
    Backends,
    /// Write a small showcase world (terrain, cottage, lake, mobs) for trying things out.
    DemoWorld {
        dir: PathBuf,
        /// Instead of the showcase, line up every bundled entity model on a platform.
        #[arg(long)]
        lineup: bool,
    },
}

#[derive(Subcommand)]
enum AssetsCmd {
    /// Download a client jar from Mojang into the cache ("latest" for the latest release).
    Fetch { version: String },
    /// Show which jar would be used for a version.
    Which { version: Option<String> },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(&cli.log))
        .format_timestamp_millis()
        .init();
    if let Some(n) = cli.threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build_global()?;
    }
    match cli.cmd {
        Cmd::Render(args) => {
            let rep = job::run(&args)?;
            println!(
                "wrote {} ({}x{}) with backend {}",
                rep.output.display(),
                rep.width,
                rep.height,
                rep.backend
            );
            for (name, secs) in &rep.timings {
                println!("  {name:<18} {secs:>8.3}s");
            }
            let total: f64 = rep.timings.iter().map(|t| t.1).sum();
            println!("  {:<18} {:>8.3}s", "total", total);
        }
        Cmd::Info {
            world,
            dimension,
            entities,
        } => info(&world, &dimension, entities)?,
        Cmd::Assets { cmd } => match cmd {
            AssetsCmd::Fetch { version } => {
                let p = jar::download_client_jar(&version)?;
                println!("{}", p.display());
            }
            AssetsCmd::Which { version } => {
                let p = jar::resolve_jar(&jar::JarRequest {
                    explicit: None,
                    version: version.as_deref(),
                    allow_download: false,
                })?;
                println!(
                    "{} (version {})",
                    p.display(),
                    jar::jar_version(&p).unwrap_or_else(|| "?".into())
                );
            }
        },
        Cmd::Backends => backends(),
        Cmd::DemoWorld { dir, lineup } => {
            let world = if lineup {
                let mut ids: Vec<String> =
                    lumenwright::scene::entities::EntityModels::load_builtin()
                        .models
                        .into_keys()
                        .collect();
                ids.sort();
                lumenwright::fixture::lineup(&ids, 10)
            } else {
                lumenwright::fixture::showcase()
            };
            world.write(&dir)?;
            println!("wrote showcase world to {}", dir.display());
        }
    }
    Ok(())
}

fn backends() {
    println!("cpu: available ({} threads)", rayon::current_num_threads());
    #[cfg(feature = "gpu")]
    for (name, kind) in [
        ("metal", lumenwright::render::BackendKind::Metal),
        ("vulkan", lumenwright::render::BackendKind::Vulkan),
        ("dx12", lumenwright::render::BackendKind::Dx12),
    ] {
        match lumenwright::render::gpu::probe(kind) {
            Ok(desc) => println!("{name}: available ({desc})"),
            Err(e) => println!("{name}: unavailable ({e})"),
        }
    }
    #[cfg(feature = "cuda")]
    match lumenwright::render::cuda::probe() {
        Ok(desc) => println!("cuda: available ({desc})"),
        Err(e) => println!("cuda: unavailable ({e:#})"),
    }
    #[cfg(not(feature = "cuda"))]
    println!("cuda: not compiled in (build with --features cuda)");
}

fn info(path: &std::path::Path, dimension: &str, with_entities: bool) -> Result<()> {
    let w = World::open(path, Dimension::parse(dimension)?)?;
    let l = &w.level;
    println!("name:         {}", l.name);
    println!(
        "version:      {} (data version {})",
        l.version_name.as_deref().unwrap_or("?"),
        l.data_version.unwrap_or(0)
    );
    println!("spawn:        {:?}", l.spawn);
    println!("time of day:  {}", l.day_time.rem_euclid(24000));
    if let Some(p) = &l.player {
        println!(
            "player:       {:.2}, {:.2}, {:.2} yaw {:.1} pitch {:.1} in {}",
            p.pos[0], p.pos[1], p.pos[2], p.yaw, p.pitch, p.dimension
        );
    }
    let regions = w.regions();
    println!(
        "regions:      {} in {}",
        regions.len(),
        w.region_dir().display()
    );
    if with_entities {
        for (rx, rz) in regions {
            let min = [rx * 32, rz * 32];
            let max = [rx * 32 + 31, rz * 32 + 31];
            let ents = w.load_entities(min, max)?;
            if ents.is_empty() {
                continue;
            }
            let mut counts = std::collections::BTreeMap::new();
            for e in &ents {
                *counts.entry(e.short_id().to_string()).or_insert(0) += 1;
            }
            let first = &ents[0];
            println!(
                "  r.{rx}.{rz}: {:?} (e.g. {} at {:.1}, {:.1}, {:.1})",
                counts,
                first.short_id(),
                first.pos[0],
                first.pos[1],
                first.pos[2]
            );
        }
    }
    Ok(())
}
