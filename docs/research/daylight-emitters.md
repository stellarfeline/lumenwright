# Emitters in a clear daytime frame

A clear daytime HQ frame of a town drew every lantern and torch as a saturated bright
point, so the frame read as dotted with lights in broad day; more samples converge to the
same points. This note records what Chunky, the reference HQ mode is ported from, sets for
a scene's emitters, how Lumenwright's day look departed from it, and the preset it now
takes. Each statement is marked **[cited]** (Chunky's source or documentation says it),
**[measured]** or **[authored]** (Lumenwright's own rule). Chunky is read at revision
`156e2bba2526`, the source of Chunky 2.5.0-SNAPSHOT.474 (GPL-3.0, the licence its ports
here carry; see `ACKNOWLEDGEMENTS.md`).

## What Chunky sets for a new scene

| setting | Chunky's default | read at |
|---|---|---|
| emitters | **off** (`DEFAULT_EMITTERS_ENABLED = false`) | `chunky/renderer/scene/Scene.java`, the constant and the field `emittersEnabled` |
| emitter intensity | 13 (`DEFAULT_EMITTER_INTENSITY`), applied only while emitters are on | `Scene.java` |
| emitter sampling | `NONE` | `Scene.java`, the field `emitterSamplingStrategy` |
| sun | intensity 1.25 (`Sun.DEFAULT_INTENSITY`), altitude π/3, azimuth π/2.5 | `chunky/renderer/scene/sky/Sun.java` |
| sky light | 1 (`Sky.DEFAULT_INTENSITY`, for the sky's exposure and its light modifiers) | `chunky/renderer/scene/sky/Sky.java` |
| exposure | 1 (`DEFAULT_EXPOSURE`) | `Scene.java` |
| post-processing | gamma correction, gamma 2.2 (`DEFAULT_POSTPROCESSING_FILTER`, `DEFAULT_GAMMA`) | `Scene.java` |

[cited, all rows] So Chunky's default scene is a daylight scene, the sun 60° high, with its
emitters off. In `PathTracer.doDiffuseReflection` a surface adds emitted light, and emitter
sampling runs, only under `scene.emittersEnabled`; the emitted light is the texel colour
squared times the material's emittance times `emitterIntensity` [cited]. Chunky's lighting
reference (chunky-dev.github.io, *Render Controls – Lighting*) says emitters, when enabled,
"contribute lighting to the scene" and that the intensity "is a multiplier of the base
emittance value of each material"; it gives no default and no advice on day or night use
[cited]. A Chunky scene with emitters on is the scene's own choice: the benchmark scene of
[benchmark.md](../benchmark.md) states `"emittersEnabled": true` [cited,
`docs/bench/treehouse-camp-bench.json`].

## How Lumenwright's day look departed

HQ mode's day look, Chunky's look on the client's day plateau (ticks 730 to 11270, where the
client's `visual/sky_light_factor` holds its day value), took Chunky's sun 1.25, sky 1,
exposure 1 and gamma tone map, all Chunky's defaults, and drew emitters at intensity 13
with sampling `ONE` [cited, `request.rs` and `sky.rs` at `c3a8500`]. The emitters were the
one setting of the day look that was not Chunky's default. A lantern texel of linear colour
`c` emitted `13 c²` against a sunlit face's `1.25^2.2 c cos θ ≈ 1.63 c cos θ`, so a bright
texel was some eight times a sunlit face of its colour; at a distance a lantern smaller than
a pixel still carried its pixel [authored arithmetic, from the cited values].

The low-light presets were tuned with emitters on: the game look's intensity 137.9628 was
fitted at dusk ([game-sky.md](game-sky.md), *Lanterns*), and a rain or thunder frame is a
stated sky at no tick, Chunky's look at 13 under a sky dimmed to the weather's light. The
clear daytime presets were never tuned for emitters [cited, the record of the fit].

## The preset

[authored] HQ mode's default emitter intensity (`request::default_emitter_intensity`) is
0.06, `DAY_EMITTER_INTENSITY`, when the frame stands at a tick on the client's day plateau:
a vanilla sky, or a stated sky at a tick, whose `visual/sky_light_factor` is at its day
value. It is the level at which a lit emitter in full daylight is drawn as its texture lit
by the sun and the sky, as a block of the same texture beside it is (see *Measurements*),
and a room the sky cannot reach is still lit, dimly, by its lamps. Everywhere else it is
each look's own intensity mixed by the day weight, and 13 for a stated sky at no tick. A
stated `--emitter-intensity` (`HqOptions::emitter_intensity`) is used as given. No
algorithm changed; the sun, the sky, the exposure and the tone map are untouched.

Chunky's default scene has its emitters off, so a closed room in it is black at any hour
[cited, the table above]. The game lights such a room by block light at any hour, and in
full daylight draws a lantern as its texture, where the lightmap clamps block light and sky
light together and a sunlit face is already at full brightness [cited: the 1.21.11
client's lightmap; recorded here only as agreement, not as the rule]. The preset keeps the
second and gives the room a little of the first.

### What the preset gives up

* **An interior on the day plateau is lit dimly.** At 0.06 a lamp lights a closed room at
  under a two-hundredth of the night's intensity (137.9628): the gallery hall below has a
  mean display brightness of 2.68/255 at noon against 29.28/255 at 13. A frame that wants
  the room bright states `--emitter-intensity`.
* **The default intensity steps at the day plateau's edges.** Off the plateau the intensity
  is the low-light presets' own, unchanged, so at ticks 730 and 11270 it steps between 0.06
  and the mixed value (about 13 there). The sky has no step; the emitters do. One tick
  across 11270 moves the showcase frame (CPU, 8 spp, 96×54) by 0.464/255 with default
  emitters, and by 0.057/255 with the emitters held at 13; one tick inside Chunky's look
  moves it by 0.040/255 [measured, the test `the_default_sky_follows_the_day_without_a_jump`].
  With the plateau at 0 the same step measured 4.043/255: at 0 emitter sampling is off, so
  the two frames' random streams part as well as their intensity [measured].

## Measurements

The daylight-emitters fixture (`demo-world --daylight-emitters`): a lit and an unlit
furnace, whose tops are one texture, a lantern, glowstone beside sandstone and a torch on
smooth stone in the open.

### The level

At noon (`--time 6000`, CPU, 64 spp, 192×108, the frame of the test
`a_daylight_emitter_is_drawn_as_its_sunlit_neighbour`), the largest channel gap between the
7×7 means of the lit and the unlit furnace tops, by stated `--emitter-intensity`
[measured]:

| intensity | 0 | 0.01 | 0.02 | 0.05 | **0.06** | 0.07 | 0.08 | 0.1 | 0.2 | 0.4 | 0.8 | 1.625 | 13 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| gap (/255) | 2.57 | 2.43 | 2.53 | 2.78 | **2.86** | 3.00 | 3.06 | 3.14 | 3.94 | 5.59 | 8.73 | 15.16 | 70.14 |

The test's tolerance is 3/255. A 7×7 mean moves in steps of 1/49, so 0.07 sits on the
tolerance itself (147/49) and 0.08 is outside; 0.06 is the largest hundredth inside it.
Of the 2.57/255 at 0, 1.63/255 remains at 1024 spp (lit (167.8, 178.4, 191.0), unlit
(166.3, 176.8, 189.3)): the two tops differ by where they fall on the pixel grid, not by
noise alone [measured].

A second instrument, the lit top against itself: at 1024 spp, how far the lit top's mean
moves from its value at intensity 0. It moves 1.8/255 at 0.2 and 3.4/255 at 0.4 (red
channel) [measured], so in proportion about 0.5/255 at 0.06 [authored arithmetic]. The two
instruments agree that 0.06 adds less than the tolerance; the first, which carries the
two tops' own 1.6 to 2.6/255 difference, is the one that binds.

The level is far below the estimate an eighth of 13 (about 1.6), which equates a white
texel's emitted `13 c²` with a sunlit face's `1.63 c`. An emitter is lit by the sun as well
as emitting, so any intensity adds to its sunlit brightness: at 1.625 the lit top is
15.16/255 brighter than the unlit one. The furnace top's own emission reaches the unlit
top's sunlit brightness only near intensity 10: with the sun and the sky off
(`--sky chunky --sun-intensity 0 --sky-intensity 0`) the lit top's mean is 54.7/255 at
0.8, 75.5 at 1.625 and 103.4 at 3.25, against 171 for the unlit top in the sun [measured].

### A sealed room

The lantern room (`fixture::lantern_room`): a closed 9 × 9 stone-brick room, 5 high, with
no opening to the sky, lit only by three lanterns, at noon. Its frame's mean display
brightness (CPU, 64 spp, 192×108) is 0.015/255 with emitters at 0 and 1.922/255 by default
[measured, the test `a_sealed_room_is_lit_by_its_lanterns_at_noon`, floor 1/255]. The torch
room (`demo-world --torch-room`) at noon: 0.01/255 at 0, 2.20 at 0.08, 4.33 at 0.36, 8.51
at 1.6 and 21.99 at 13 [measured, CPU, 64 spp, 192×108].

### At night

At midnight the default frame is the frame at 137.9628 byte for byte, and the floor by the
lit furnace (237.7, 229.6, 215.3) is far above the floor by the unlit one (89.1, 85.4,
101.0) [measured, the test `a_night_emitter_still_lights_its_surroundings`].

The comparison frames are in `docs/renders/`: `daylight-emitters-noon-before.png` (emitter
intensity 13) and `daylight-emitters-noon.png` (the default), and
`daylight-emitters-midnight.png`, each drawn from the fixture by

```sh
target/release/lumenwright demo-world --daylight-emitters yard
target/release/lumenwright render -w yard --mode hq --backend cpu --spp 256 --width 384 --height 216 \
    --camera 0.5,72,7.5 --look-at 0.5,65,-0.5 --fov 55 --time 6000 -o daylight-emitters-noon.png
```

with `--emitter-intensity 13` for the frame before, and `--time 18000` for midnight.

### Eighteen stated skies

Delvewright's engine states a frame's sky from a time and a weather: six ticks (1000, 6000,
12000, 13000, 18000, 23000) under a clear sky as a stated sky at that tick by the time of
day, and under rain or thunder as a stated sky at no tick (Chunky's look, its sun dark, its
sky scaled to the weather's light). Each of the 18 was drawn on the daylight-emitters yard
through that engine's frame path (`delvec` 1.16.0, `frames::draw`; Metal, Apple M2 Pro,
384×216, 256 spp) with the day plateau at 0 and at 0.06 [measured; SHA-256 of the RGBA
bytes]:

* The 16 skies off the clear day plateau (clear at 12000, 13000, 18000 and 23000; rain and
  thunder at all six ticks) are byte-identical: 16 of 16 frames. A second run at 0
  reproduced all 18 frames' bytes.
* The 2 clear skies on the plateau differ: the frame's mean display brightness moves from
  163.798 to 163.850 at tick 1000 and from 214.330 to 214.366 at 6000.
* Rain and thunder at ticks 1000 and 6000 keep emitters at 13 under a daytime sky dimmed to
  the weather's light; there a lantern is still a bright point and the lit furnace's top
  clips to white. They are left as they were tuned.

### Delvewright's frames

Through the same engine (`delvec` 1.16.0, Metal, Apple M2 Pro), with the day plateau at 0
and at 0.06 [measured]:

* The gallery's showcase camera `hall-from-its-corner`, a sealed hall lit only by lanterns
  at tick 6000 (1600×900): mean display brightness 0.000/255 at 0 and 2.680/255 at 0.06
  (29.28/255 at 13). The same world and pose through `lumenwright render` (vanilla sky,
  CPU, 64 spp, 400×225): 2.307/255 at 0.06, 5.403 at 0.4, 10.150 at 1.625, 24.732 at 13.
* A lantern-lit island town seen from the sea at tick 1000, clear (Delvewright's content
  campaign `foot-of-the-mountain`, its showcase pose `salvage-ground`, the load world drawn
  through `frames::draw` at 1920×1080, 256 spp): the written world is byte-identical
  between the two runs (its SHA-256 the same); the frame's mean display brightness moves
  from 124.444/255 to 124.449/255, 416 792 of 2 073 600 pixels move, by 0.443/255 on
  average, and 51 by 20/255 or more, the most by 36/255. Every one of those 51 is a dark
  pixel made warmer (the largest, (25, 28, 28) to (61, 43, 27), a doorway under an eave);
  none is a lamp drawn as a bright point. Side by side the lanterns read as their textures
  in both frames.
