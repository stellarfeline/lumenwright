//! Command-line values for the library's option enums.
//!
//! Each type mirrors one library enum variant for variant, carries the flag's
//! help text, and converts both ways with exhaustive matches, so a variant added
//! on either side fails to compile until the other side has it.

use clap::ValueEnum;
use lumenwright::render::post::Tonemap;
use lumenwright::render::{BackendKind, Mode};
use lumenwright::request::{EmitterSampling, UnknownEntities};

macro_rules! mirror {
    ($(#[$m:meta])* $cli:ident => $lib:ident { $($(#[$vm:meta])* $v:ident),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
        pub enum $cli {
            $($(#[$vm])* $v),+
        }

        impl From<$cli> for $lib {
            fn from(v: $cli) -> $lib {
                match v {
                    $($cli::$v => $lib::$v),+
                }
            }
        }

        impl From<$lib> for $cli {
            fn from(v: $lib) -> $cli {
                match v {
                    $($lib::$v => $cli::$v),+
                }
            }
        }
    };
}

mirror! {
    /// `--mode`.
    ModeArg => Mode {
        /// Vanilla-client look: rasterized, no ray tracing.
        Quick,
        /// Chunky-like path tracing.
        Hq,
    }
}

mirror! {
    /// `--backend`.
    BackendArg => BackendKind {
        /// Best available: Metal/Vulkan/DX12 GPU, then CPU.
        Auto,
        Cpu,
        /// Any native GPU API wgpu can find.
        Gpu,
        Metal,
        Vulkan,
        Dx12,
        Cuda,
    }
}

mirror! {
    /// `--tonemap`.
    TonemapArg => Tonemap {
        /// Clamp and the display gamma 2.2 (Chunky's default, "gamma correction").
        Gamma,
        /// ACES filmic fit (Narkowicz), then the display gamma.
        Aces,
        /// Hable / Uncharted 2 filmic, then the display gamma.
        Filmic,
        /// Hable read brighter so linear 0.18 shows as the game shows it (0.18^(1/2.2)),
        /// then the display gamma.
        FilmicMidGrey,
    }
}

/// `--sky`: the library's [`lumenwright::request::SkyLook`]; the looks' values
/// come from their own flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum SkyArg {
    /// By the time of day: Chunky's sky and sun on the client's day plateau, the game's sky
    /// at night, mixed between them by the client's `visual/sky_light_factor`.
    Auto,
    /// The game's own sky at the tick, lit as the client lights it: the lightmap's sky
    /// light, no sunlight, the client's sky, sun, moon and stars drawn.
    Game,
    /// Chunky's simulated sky: the Preetham sky and a sun that lights and shadows.
    Chunky,
}

mirror! {
    /// `--emitter-sampling`.
    EmitterSamplingArg => EmitterSampling {
        /// None: emitters light the scene only where a path meets them.
        None,
        /// One face of one nearby emitter.
        One,
        /// Every face of one nearby emitter.
        OneBlock,
        /// Every face of every nearby emitter.
        All,
    }
}

mirror! {
    /// `--unknown-entities`.
    UnknownEntitiesArg => UnknownEntities {
        /// A box the size of the entity's hitbox.
        Box,
        /// Nothing; the kind is counted in the entity statistics.
        Skip,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_value_converts_both_ways() {
        for v in ModeArg::value_variants() {
            assert_eq!(ModeArg::from(Mode::from(*v)), *v);
        }
        for v in BackendArg::value_variants() {
            assert_eq!(BackendArg::from(BackendKind::from(*v)), *v);
        }
        for v in EmitterSamplingArg::value_variants() {
            assert_eq!(EmitterSamplingArg::from(EmitterSampling::from(*v)), *v);
        }
        for v in TonemapArg::value_variants() {
            assert_eq!(TonemapArg::from(Tonemap::from(*v)), *v);
        }
        for v in UnknownEntitiesArg::value_variants() {
            assert_eq!(UnknownEntitiesArg::from(UnknownEntities::from(*v)), *v);
        }
    }
}
