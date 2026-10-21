# The command line

[Lumenwright](../README.md)

## Command line (render)

| flag | default | meaning |
|---|---|---|
| `-w, --world` | required | save directory (contains `level.dat`) |
| `-o, --output` | render.png | the PNG to write |
| `--mode quick\|hq` | quick | vanilla raster or path tracing |
| `--backend auto\|cpu\|gpu\|metal\|vulkan\|dx12\|cuda` | auto | `auto` uses the GPU if one is available, otherwise the CPU |
| `--camera x,y,z` | player eye, else spawn | eye position |
| `--yaw`, `--pitch` | player's | Minecraft convention: yaw 0 = south, 90 = west; pitch > 0 looks down |
| `--look-at x,y,z` | | overrides yaw and pitch |
| `--fov` | 70 | vertical field of view in degrees |
| `--width`, `--height` | 1280×720 | output size |
| `--render-distance` | 8 | chunk radius loaded around the camera |
| `--dimension` | overworld | `overworld`, `nether`, `end` |
| `--time` | world time | ticks: 0 = sunrise, 6000 = noon |
| `--sun-altitude`, `--sun-azimuth` | from time, 135 | degrees (azimuth 0 = north, 90 = east); an altitude overrides `--time` for the sun |
| `--spp`, `--max-bounces`, `--seed` | 64, 4, 0 | HQ sampling; the same seed gives the same image |
| `--sky auto\|game\|chunky` | auto | HQ: by the time of day (Chunky's sky and sun on the client's day plateau, the game's sky on the night plateau, mixed between by the client's `visual/sky_light_factor`), or either look at every hour |
| `--exposure`, `--tonemap gamma\|aces\|filmic\|filmic-mid-grey` | 1.0, each look's | HQ post-processing, Chunky's filters (each ends in the display gamma 2.2); `filmic-mid-grey` is Hable read brighter so linear 0.18 shows as the game shows it. Unstated, each look takes its own (`gamma` for Chunky's, `filmic-mid-grey` for the game's), mixed in display values under `auto` |
| `--sun-intensity`, `--sky-intensity` | 1.25, 1 | HQ, Chunky's sky (`chunky`, `auto`): Chunky's sun intensity (irradiance intensity^2.2) and sky light; refused under `--sky game` |
| `--emitter-intensity` | each look's | HQ: the emitter intensity; an emitting surface emits its texel colour squared times its block's emittance times this (0 turns emitters off). Unstated: 13 (Chunky's) for Chunky's look and 137.9628 for the game's, authored by Lumenwright (fitted to Delvewright's engine's render at dusk), mixed linearly under `auto` |
| `--emitter-sampling none\|one\|one-block\|all`, `--prevent-normal-emitter` | one | HQ: Chunky's emitter sampling strategies, and its "prevent normal emitter with sampling" |
| `--ssaa N` | 1 | quick-mode supersampling |
| `--mipmap-levels N` | 4 | quick-mode mip levels for block textures, 0 (off) to 4, as the client's option |
| `--brightness`, `--flat-lighting` | 0.5 | vanilla brightness slider, and smooth lighting off |
| `--hdr-output out.pfm` | | also write the linear HDR image (HQ) |
| `--no-entities`, `--players`, `--unknown-entities box\|skip` | | entity options |
| `--no-block-entities` | | leave out block entities drawn by Java code |
| `--minecraft-jar`, `--mc-version`, `--unifont`, `--offline`, `--resource-pack` | | the client jar, the Unifont and packs (see [Textures](textures.md#textures-the-chunky-approach)) |
| `--threads`, `--log` | all cores, info | CPU worker threads and log level, for every subcommand |
