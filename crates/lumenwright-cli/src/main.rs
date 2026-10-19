use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use lumenwright::world::{Dimension, World};
use lumenwright_cli::jar;
use lumenwright_cli::job::{self, RenderArgs};

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
        /// List block entities (id, position, block, sign text) in the chunk box
        /// "x1,z1,x2,z2" (block coordinates).
        #[arg(long, value_name = "X1,Z1,X2,Z2", allow_hyphen_values = true)]
        block_entities: Option<String>,
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
        /// Instead of the showcase, one of every block entity Lumenwright draws.
        #[arg(long, conflicts_with = "lineup")]
        block_entities: bool,
        /// Instead of the showcase, one of every way Lumenwright draws an item.
        #[arg(long, conflicts_with_all = ["lineup", "block_entities"])]
        items: bool,
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
            block_entities,
        } => info(&world, &dimension, entities, block_entities.as_deref())?,
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
        Cmd::DemoWorld {
            dir,
            lineup,
            block_entities,
            items,
        } => {
            let world = if items {
                lumenwright_cli::fixture::item_gallery(true)
            } else if block_entities {
                lumenwright_cli::fixture::block_entity_gallery()
            } else if lineup {
                let mut ids: Vec<String> =
                    lumenwright::scene::entities::EntityModels::load_builtin()
                        .models
                        .into_keys()
                        .collect();
                ids.sort();
                lumenwright_cli::fixture::lineup(&ids, 10)
            } else {
                lumenwright_cli::fixture::showcase()
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

fn info(
    path: &std::path::Path,
    dimension: &str,
    with_entities: bool,
    block_entities: Option<&str>,
) -> Result<()> {
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
    if let Some(rect) = block_entities {
        let v: Vec<i32> = rect
            .split(',')
            .map(|p| p.trim().parse::<i32>())
            .collect::<std::result::Result<_, _>>()
            .map_err(|_| anyhow::anyhow!("expected \"x1,z1,x2,z2\", got \"{rect}\""))?;
        let [x1, z1, x2, z2] = <[i32; 4]>::try_from(v)
            .map_err(|_| anyhow::anyhow!("expected \"x1,z1,x2,z2\", got \"{rect}\""))?;
        let (lo, hi) = ([x1.min(x2), z1.min(z2)], [x1.max(x2), z1.max(z2)]);
        let area = w.load_area([lo[0] >> 4, lo[1] >> 4], [hi[0] >> 4, hi[1] >> 4], false)?;
        let list = lumenwright::scene::block_entities::parse_all(&area.block_entities);
        let inside: Vec<_> = list
            .iter()
            .filter(|b| (lo[0]..=hi[0]).contains(&b.pos[0]) && (lo[1]..=hi[1]).contains(&b.pos[2]))
            .collect();
        println!(
            "block entities: {} in x {}..{}, z {}..{}",
            inside.len(),
            lo[0],
            hi[0],
            lo[1],
            hi[1]
        );
        for b in inside {
            let state = area
                .registry
                .state(area.volume.block(b.pos[0], b.pos[1], b.pos[2]));
            println!(
                "  {} at {},{},{}  {state}",
                b.short_id(),
                b.pos[0],
                b.pos[1],
                b.pos[2]
            );
            if let Some([front, back]) = b.sign_lines(&lumenwright::assets::lang::Lang::default()) {
                for (side, lines) in [("front", front), ("back", back)] {
                    if lines.iter().any(|l| !l.is_empty()) {
                        println!("    {side}: {:?}", lines);
                    }
                }
            }
        }
    }
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
