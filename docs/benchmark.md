# Benchmark

[Lumenwright](../README.md)

## Design

The design was written before any run. Every number in the tables comes from this design;
a cell that cannot be measured is marked n/a with its reason.

### Instruments

- Lumenwright: the `lumenwright` command built from `f291590` (the code head of
  `feat/lw-game-sky`; later commits on the branch are documentation), `--release`; on the PC
  with `--features cuda` and the PTX from `scripts/build-ptx.sh` at the same revision.
- Chunky: `chunky-core-2.5.0-SNAPSHOT.474.g156e2bb.jar` (sha256
  `79ea46233cfa4929e738c6074ff8aabf4f8a2e59cfe9925abc3888a782bad15c`) and its libraries,
  run headless (`-render <scene> -f`), the client jar 1.21.11 (sha1
  `ba2df812c2d12e0219c489c4cd9a5e1f0760f5bd`) as its resources. JDK 17 on the Mac, JDK 21 on
  the PC (what each machine has).
- World: the Treehouse Camp save the README's renders use (region files' sha256 list hashes
  to `526fa9df…`, the same on both machines).
- Machines: Mac = Apple M2 Pro (10 CPU cores, 16 GPU cores), macOS; PC = AMD Ryzen 7 7800X3D
  (16 threads) and RTX 4070 Ti SUPER, Linux under WSL2.
- Timers, both recorded for every run: the renderer's own (Chunky's `renderTime`;
  Lumenwright's `render` stage) and the whole command's wall clock (Python `time.time()` on
  the Mac, `/proc/uptime` on the PC), which includes reading the world and, for Chunky,
  starting the JVM and building its octree.
- Runs: three per cell, the median reported with the three values. In question 1 the
  renderers alternate run by run (Chunky, then each Lumenwright backend, three rounds), so a
  load change falls on all of them alike.
- Load: the one-minute load average and the three busiest processes are recorded before every
  run. Other workers share the Mac; a run is not discarded for load, it is reported with it.

### The frames

- **F-chunky**: the whole-camp view, camera 140, 118, 14 looking at 56, 82, 58, vertical
  field of view 60°, 1280 × 720, 625 chunks (`--render-distance 12`), four bounces (Chunky
  `rayDepth` 5), Chunky's Preetham sky and a sun at altitude 45° from the south-east (Chunky
  altitude π/4, azimuth π/4), emitters at intensity 13 sampled `ONE`, gamma tone map,
  250 samples per pixel. Lumenwright: `--sky chunky --tonemap gamma --time 4000
  --sun-altitude 45 --sun-azimuth 135 --spp 250 --max-bounces 4`. Chunky:
  `docs/bench/treehouse-camp-bench.json`. It is the one frame both renderers draw by the same
  algorithm (Chunky cannot draw the game's sky with two maps).
- **F-game**: the same camera, resolution, chunks and bounces under Lumenwright's defaults
  at `f291590` (the game's sky, the mid-grey tone curve, emitters 13 `ONE`), at tick 4000
  (day) and 12800 (dusk). From 0.2.0 that frame is `--sky game` (the default goes by the time
  of day), and the game look's emitter intensity is 137.9628; an emitter's intensity scales
  the light it adds and not the work, so the times stand.
- **F-quick**: the same camera, resolution and chunks in quick mode at tick 4000.

### Questions

1. **Lumenwright against Chunky on the same machine.** Changes: the renderer (Chunky on the
   CPU; Lumenwright on each backend the machine has). Fixed: F-chunky (frame, camera,
   resolution, 250 spp, four bounces, sky mode, world), the machine; load recorded.
   Mac: Chunky CPU, Lumenwright Metal, Lumenwright CPU. PC: Chunky CPU, Lumenwright CUDA,
   Lumenwright CPU, Lumenwright Vulkan if WSL2 offers a hardware Vulkan adapter.
2. **Lumenwright's backends against each other.** Changes: the backend. Fixed: the frame
   (F-chunky for HQ, the runs of question 1; F-quick for quick mode), the machine; load
   recorded. HQ: as question 1. Quick: Mac CPU and Metal; PC CPU (the CUDA backend has no
   rasterizer and uses the CPU's).
3. **How cost scales with samples.** Changes: spp, at 64, 256 and 1024. Fixed: F-game
   (camera, resolution, bounces, sky mode), the backend (Metal on the Mac, CUDA on the PC),
   the machine; day and dusk each run at every spp, so they compare at the same spp; load
   recorded. The renders' quality budgets (4096 and 16384 spp in the README) are not speed
   cells.

## Measured on an M2 Pro (10 CPU cores, 16 GPU cores)

Other work shared the Mac during every session: the one-minute load average before the runs
was 16 to 251 (Spotlight's indexer and another job's compiler and engine), on a 10-core
machine. The Metal runs barely move with it (32.0 to 33.8 s over six runs); the CPU runs of
Chunky and of Lumenwright do (131.6 to 289.7 s), so the Mac's CPU rows are not a measure of
either renderer's CPU speed: the PC's, under a load of 8 to 16 on 16 threads, are.

### Question 1 and 2 (HQ): F-chunky

Session 1 (15:11 to 15:46):

| machine | renderer | own timer, median (runs) s | whole command, median (runs) s | 1-min load before each run |
|---|---|---|---|---|
| Mac | Chunky, CPU | 267.56 (289.74, 209.34, 267.56) | 275.55 (301.44, 213.62, 275.55) | 21.73, 123.60, 142.03 |
| Mac | Lumenwright, Metal | 33.54 (33.54, 33.81, 32.03) | 33.98 (33.98, 34.86, 32.52) | 156.00, 99.89, 251.06 |
| Mac | Lumenwright, CPU | 211.37 (211.37, 227.01, 146.87) | 211.83 (211.83, 227.35, 147.24) | 149.96, 96.09, 233.06 |
| PC | Chunky, CPU | 78.60 (93.18, 77.60, 78.60) | 82.03 (97.69, 81.50, 82.03) | 8.01, 14.40, 14.38 |
| PC | Lumenwright, CUDA | 6.14 (6.15, 6.05, 6.14) | 6.74 (6.74, 6.64, 6.74) | 13.98, 15.35, 15.66 |
| PC | Lumenwright, CPU | 54.84 (54.67, 54.84, 55.05) | 55.14 (54.97, 55.14, 55.34) | 11.98, 13.14, 13.40 |

Session 2 (15:49 to 16:12), the same design run again:

| machine | renderer | own timer, median (runs) s | whole command, median (runs) s | 1-min load before each run |
|---|---|---|---|---|
| Mac | Chunky, CPU | 208.31 (179.02, 208.31, 277.95) | 212.33 (183.00, 212.33, 285.99) | 16.37, 74.67, 96.94 |
| Mac | Lumenwright, Metal | 32.61 (32.34, 32.61, 32.93) | 33.08 (32.70, 33.08, 33.47) | 86.59, 84.84, 77.58 |
| Mac | Lumenwright, CPU | 212.86 (131.59, 229.03, 212.86) | 213.39 (132.02, 229.58, 213.39) | 73.67, 75.15, 67.34 |

### Question 2 (quick): F-quick

| machine | backend | own timer, median (runs) s | whole command, median (runs) s | 1-min load |
|---|---|---|---|---|
| Mac | Metal | 0.05 (0.06, 0.04, 0.05) | 0.33 (0.36, 0.33, 0.33) | 204.90, 204.90, 204.90 |
| Mac | CPU | 0.11 (0.12, 0.11, 0.09) | 0.39 (0.39, 0.40, 0.37) | 204.90, 204.90, 204.90 |
| PC | CPU | 0.14 (0.14, 0.14, 0.13) | 0.48 (0.47, 0.48, 0.48) | 13.84, 13.84, 13.84 |

The quick mode's own timer is the `render` stage (rasterizing); the whole command adds
reading the world and the jar and building the mesh. The CUDA backend has no rasterizer (n/a).

### Question 3: F-game against samples

| spp | Mac Metal, day | Mac Metal, dusk | PC CUDA, day | PC CUDA, dusk |
|---|---|---|---|---|
| 64 | 5.21 (5.53) | 5.21 (5.54) | 1.07 (1.67) | 1.07 (1.69) |
| 256 | 21.29 (22.24) | 21.50 (21.89) | 4.12 (4.73) | 4.12 (4.74) |
| 1024 | 83.11 (83.45) | 83.50 (83.86) | 16.31 (16.91) | 16.48 (17.10) |

## Measured on an RTX 4070 Ti SUPER (driver 616.92, WSL2)

The PC rows of the tables above: AMD Ryzen 7 7800X3D (16 threads), RTX 4070 Ti SUPER, driver
616.92, Linux 6.6.87.2-microsoft-standard-WSL2, JDK 21 for Chunky. Lumenwright on Vulkan: n/a
(WSL2 offers only a software Vulkan adapter, which `lumenwright backends` refuses). Metal and
DX12: n/a on this machine.

### Answers

* (1) On the PC, under the same frame, Lumenwright's CUDA backend renders F-chunky 12.8 times
  as fast as Chunky on the CPU by the renderers' own timers (78.60 / 6.14 s) and 12.2 times by
  the whole command (82.03 / 6.74 s); Lumenwright's CPU backend 1.43 and 1.49 times. On the Mac,
  Lumenwright on Metal is 6.4 to 8.0 times as fast as Chunky by its own timer in the two loaded
  sessions (208.31 / 32.61, 267.56 / 33.54 s); the CPU comparison there is not taken (above).
* (2) HQ on the PC: CUDA 6.14 s, CPU 54.84 s. On the Mac: Metal 32.6 to 33.5 s. Quick mode:
  Metal 0.05 s and CPU 0.11 s on the Mac, CPU 0.14 s on the PC, each about 0.3 to 0.5 s end to
  end.
* (3) Cost is linear in samples past a fixed overhead, and day and dusk cost the same under
  the game's sky (the sun casts no light, so no shadow rays are traced at any hour): on Metal
  5.2, 21.3 and 83.1 s at 64, 256 and 1024 spp (about 81 ms a sample), on CUDA 1.07, 4.12 and
  16.3 s (about 16 ms a sample).

The frames: each Lumenwright F-chunky frame on Metal is byte-identical to the frame the
previous benchmark measured (Lumenwright at `0b39f8f`), so `--sky chunky` draws what Chunky's
sky drew before the game's sky existed; against Chunky's frame converged at 4000 samples it is
display mean 1.66/255, SSIM 0.9594, and a Chunky 250-sample frame of this session 1.99/255,
SSIM 0.9575.

## HQ mode and Chunky on one frame

F-chunky: the whole-camp view of The Treehouse Camp at 1280×720, 250 samples per pixel, four bounces, with Chunky's default sky,
sun, emitters and tone map, which Lumenwright draws under `--sky chunky` (the full method: [research/chunky-parity.md](research/chunky-parity.md)).
Against Chunky's frame converged at 4000 samples, Lumenwright's 250-sample frame differs by a
mean of 1.66/255 (SSIM 0.959), Chunky's own 250-sample frame by 1.99/255 (SSIM 0.958). At
dusk (`--time 12800`, the sun just below the horizon) the same view differs by 1.21/255
(SSIM 0.955), Chunky's own 250-sample frame by 1.28/255 (SSIM 0.954); those dusk figures were
measured at `0b39f8f`, whose Chunky sky `--sky chunky` draws unchanged.

```sh
# Lumenwright (--backend metal on the M2 Pro, cuda on the 4070)
target/release/lumenwright render -w the-treehouse-camp --camera 140,118,14 --look-at 56,82,58 --fov 60 --render-distance 12 \
    --sky chunky --tonemap gamma --time 4000 --sun-altitude 45 --sun-azimuth 135 --mode hq --spp 250 --max-bounces 4 --backend metal -o lw.png
# Chunky: the pinned core on its own classpath, as Delvewright's validation/chunky.sh runs it, without its denoise step
mkdir -p scenes && cp docs/bench/treehouse-camp-bench.json scenes/
java -Djava.awt.headless=true -Dchunky.home=$HOME/.chunky -cp "<the pinned core and its libraries>" se.llbit.chunky.main.Chunky \
    -scene-dir scenes -render treehouse-camp-bench -f
```

The Chunky scene ([`docs/bench/treehouse-camp-bench.json`](bench/treehouse-camp-bench.json); its world path is read from the working directory) states the same 625 chunks, the camera
at 140, 118, 14 with Chunky's yaw 0.483 rad and pitch −1.208 rad (looking at 56, 82, 58), a 60°
vertical field of view, `sppTarget` 250, `rayDepth` 5 (four bounces after the first hit,
read off the pinned core's `PathTracer`), `emittersEnabled`, `emitterSamplingStrategy` `ONE`,
and the sun at altitude π/4 and Chunky azimuth π/4 (towards +X +Z).
