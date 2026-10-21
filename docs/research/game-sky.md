# HQ mode's game sky: the light and the sky of the client at a tick

HQ mode's game sky (`--sky game`, `SkyLook::Game`) lights a frame as the 1.21.11 client
lights the same tick and shows the client's own sky. HQ mode's default (`--sky auto`,
`SkyLook::ByTime`) draws it at dusk and night and Chunky's sky and sun by day
([By the time of day](#by-the-time-of-day)). The method is the one Delvewright's
engine lights its Chunky scenes with (stellarfeline/delvewright at `0fd300da7`:
`crates/delvec/src/compiler/view/scene.rs`, `sky_image.rs`, and the record in its
`docs/reference/compiler.md` §4). Lumenwright ports the method, not a tuned constant: every
value is read from the client jar at the frame's tick. Chunky's Preetham sky and sun stay as
`--sky chunky` ([chunky-parity.md](chunky-parity.md)).

The client read is the 1.21.11 jar (`client.jar` sha1 `ba2df812c2d12e0219c489c4cd9a5e1f0760f5bd`)
through Mojang's mappings (`client.txt` sha1 `031a68bebf55d824f66d6573d8c752f0e1bf232a`).
Each statement is marked **[cited]** (the client, Chunky or the engine says it),
**[measured]** or **[authored]** (Lumenwright's or the engine's own rule).

## The method

| What | Lumenwright | Source |
|---|---|---|
| The sun's light | none: the kernel draws no next-event sample for a sun of colour 0 | [cited] nothing in the client's lightmap depends on the sun's direction; a cell open to the sky is drawn at the lightmap's colour whatever the hour's sun, and no block casts a sun shadow |
| The light the sky casts | the colour `lightmap.fsh` gives sky light 15 and block light 0: `SkyFactor` = `visual/sky_light_factor`, `SkyLightColor` = `visual/sky_light_color` (both from the jar's day timeline, `EnvironmentState`), `AmbientLightFactor` 0, the `gamma` option's default 0.5 as `BrightnessFactor`; read at its 8-bit level (the lightmap is an RGBA8 texture) and decoded with `x^2.2` (`sky::lightmap_sky`) | [cited] the shader, `LightTexture.updateLightTexture` (reads exactly those two attributes), `Options` (`gamma` 0.5), `TextureFormat.RGBA8`; [authored, the engine's] reading the decoded colour as the radiance a top face is lit by: a Lambertian face open to a sky of mean radiance `L` returns `albedo × L`, so under the gamma transfer it shows `albedo_display × L_display`, the game's product |
| Where that light comes from | the client's sky with nothing drawn in it (no sun, moon or stars), times a scale, plus the least uniform veil that makes the cosine-weighted mean over the upper hemisphere exactly `L` in every channel; the scale is the largest up to 50 that keeps the veil non-negative (`sky::game_sky_maps`) | [authored, the engine's `sky_image::sky_map`]; the bound 50 is Chunky's `Sky.MAX_INTENSITY` [cited] |
| What the lens sees | the client's sky at the tick, as quick mode draws it (`SkyRenderer`, `AtmosphericFogEnvironment.getBaseColor`): the clear colour, the fogged sky disc, the sunrise fan, the sun (size 30), this night's moon (size 20) and the stars, decoded with `x^2.2` | [cited] the client; Lumenwright's quick sky |
| The clear colour's lean to the sky colour | `1 − Mth.clampedLerp(min(sky fog end / 16, render distance) / 32, 0.25, 1)^0.25`; `clampedLerp(delta, start, end)`: 0.146 at 12 chunks | [cited] `getBaseColor` and `Mth.clampedLerp` bytecode |
| The sun drawn | at every altitude, as `renderSunMoonAndStars` draws it (no altitude test) | [cited] bytecode |
| The tone curve | Chunky's Hable preset read `k` = 1.463761 times brighter, so linear 0.18 shows at `0.18^(1/2.2)` (`--tonemap filmic-mid-grey`, the default under the game's sky) | [cited] Chunky's `HableToneMappingFilter` preset and arithmetic, the 18% grey card; [authored, the engine's] anchoring the curve at mid-grey, and the closed form for `k` |
| The render distance the sky is drawn for | `--render-distance` (chunks), as quick mode; the client's default 12 in the library | [cited] the client reads it in `getBaseColor` and the sky fog |
| The maps | the lens's 2048 × 1024 and the light's 360 × 180, equirectangular, bilinear, in the kernel's `sky` buffer on every backend (`integrator::sky_lens`, `sky_light`, `Params::sky_map`) | [authored] |

A camera ray, and a path that has met only glass and water, sees the lens's map; a path that
has bounced off a diffuse surface takes the light's. Chunky cannot set the two apart (its sky
modes feed one colour to the lens and to the light, scaled by `apparentSkyLight` or `skyLight`,
`Sky.getSkyDiffuseColorInner`), so the engine's lens sees the veil too.

## Where Lumenwright departs from the engine

* **The lens sees no veil** [authored]. The veil is the price of Chunky's one map: at noon,
  under a white light and a blue sky, it is the light's yellow remainder and pales the
  engine's zenith. Lumenwright's kernel reads two maps, so the lens shows the client's sky.
* **The clear colour leans 0.146 to the sky colour, not 0.218** [cited]. The engine writes
  `1 − clamp(min(512/16, 12)/32, 0.25, 1)^0.25`; the client's bytecode calls
  `Mth.clampedLerp(x/32, 0.25, 1)`, whose arguments are `(delta, start, end)`
  (`delta < 0` gives `start`, `delta > 1` gives `end`, else `lerp(delta, start, end)`), so the
  weight is `1 − (0.25 + 0.75 × 0.375)^0.25`. Lumenwright's quick sky already read it so; the
  engine's sky map, and the light it derives from it, lean too far toward the sky colour.
* **The sun is drawn below the horizon** where a ray reaches the sky there, as the client
  draws it [cited]; the engine draws Chunky's sun from −4.29° (the disc's half-width) up.
* **The sun, moon and stars are the client's** [cited]: the size-30 sun quad blended into the
  sky as the client blends it, the moon and the 1500 stars. The engine's renderer has no moon
  or stars and draws Chunky's own sun texture (a smaller square at brightness 16.3).
* **The lightmap is read at its 8-bit level** [cited]; the engine reads the shader's value.
  At tick 12800 the two differ by 0.6% in the light.
* **The lantern scale is Lumenwright's own fit.** The engine writes `emitterIntensity`
  159.003, Chunky's 13 times 12.2310, fitted so that the mean display luma of the Treehouse
  Camp's lantern-lit pixels equals a game model's. Lumenwright fits its own against the
  engine's render ([Lanterns](#lanterns)).

## By the time of day

The default look follows the client's day [cited for the quantity, authored for the mix]. On
the 1.21.11 day timeline three tracks mark one day plateau, from tick 730 to tick 11270:
`visual/sky_light_factor` and `visual/sky_light_color` hold their day values (1 and white)
and `visual/sunrise_sunset_color` its zero alpha. Off the plateau the factor falls linearly
to its night value 0.24 at 13140, holds it to 22860, and rises back by 730. The weight of
Chunky's look is where the factor at the tick stands between those two values,
`(factor − 0.24) / (1 − 0.24)` (`sky::day_weight`; the two values are read from the jar's
timeline by `environment::sky_light_factor_range`): 1 on the day plateau, 0 on the night
plateau, linear in ticks on the ramps, at most 1/1870 a tick. At the camp's dusk, tick
12800, it is 0.181818.

The two looks are mixed in one render: Chunky's sky, read through its cache, is weighted
into the game's two maps, and Chunky's sun lights and is drawn at the same weight. Light
transport is linear in the sky's and the sun's radiance, so the frame's radiance is the
weighted mix of the two looks' radiances. Each look keeps its tone curve and emitter
intensity unless the caller states one: the curves (gamma, filmic-mid-grey) are mixed in
display values and the intensities (13, 137.9628) linearly. On the day plateau the frame is
`--sky chunky`'s byte for byte; on the night plateau `--sky game`'s [measured, the test
`the_default_sky_follows_the_day_without_a_jump`]. One tick across either edge of the ramp
moves the camp frame by 0.067/255 (Metal, 64 spp, at `4d1f6bd`), as one tick inside Chunky's look does
(0.059); a hard switch at the plateau's edge moves it by 28.7 [measured].

## Lanterns

The game look's emitter intensity is 137.9628, Chunky's 13 times 10.6125: **authored by
Lumenwright**, neither a vanilla nor a Chunky value (`request::GAME_EMITTER_INTENSITY`).
Chunky's emitters light little beyond their block where the game's block light reaches 14
blocks; the scale stands in for that, as the engine's 12.2310 does. `--emitter-intensity`
takes any value over it.

The fit [measured]: the whole-camp view at tick 12800 under the default sky (day weight
0.181818), CUDA, 16384 samples, four bounces, emitters sampled `ONE` (Lumenwright's
default), rendered at intensities 13 and 26 with one seed, which draw the same random
numbers, so the frame at any intensity `k` is `HDR(13) + (k − 13)/13 × (HDR(26) − HDR(13))`
in radiance (a direct render at the fitted `k` agrees with that to 6.75e-7). Lantern-lit
pixels are those where the emitters' light at the engine's intensity 159.003 outshines the
sky's (Rec. 709 luminance): 128 564 of 921 600. The objective is that their mean display
luma under the default tone mix equals the engine's Chunky render's, 41.993; bisection gives
`k` = 115.2423 (Lumenwright reads 28.622 there at 13 and 45.956 at 159.003), so the game
look's intensity is `(115.2423 − 0.181818 × 13) / 0.818182` = 137.9628. Per pixel the fitted
frame is 7.33 luma from the reference on average (rms 9.29); fitting each quarter of the frame
on its own gives 118.5, 114.6, 116.6 and 112.6.

## Measurements

The reference is Delvewright's engine's look: the whole-camp view of *The Treehouse Camp*
(1280 × 720, camera 140, 118, 14 looking at 56, 82, 58, vertical field of view 60°, the save
the README's renders use), rendered by Chunky 2.5.0-SNAPSHOT.474 (core `156e2bb`) at 2048
samples per pixel with every key the engine writes about light and tone produced by the
engine's own functions at `0fd300da7` (`scene::sky_of`, `sky_image::sky_map` for this camera,
`scene::tone_curve`, `scene::emitter_intensity`, `EXPOSURE`, ray depth 5, emitter sampling
`NONE`). The engine's camera record cannot state these ticks (its hours are keywords and
celestial positions, and it no longer builds the released camp), so a one-file program linked
against the engine at that revision wrote the scene's keys for ticks 12800 and 4000 [measured:
at tick 12782 it writes the light `sky.color` (0.357581, 0.357581, 1), the value the engine's
`compiler.md` §4 records for the camp's setting sun]. No in-game screenshot of this camera
exists in the treehouse rounds' scratch directories.

Lumenwright renders the same frame on CUDA at 16384 samples, four bounces, with the engine's
emitter settings (`--emitter-intensity 159.003 --emitter-sampling none`) so the comparison
isolates the sky and the tone. Each component is shown on the same samples: the HDR frame
re-shown through the other tone curve by a numpy port of `post::tonemap`, which reproduces
Lumenwright's own PNGs to within one 8-bit level on 7–17 of 921 600 pixels. Display: mean
absolute 8-bit difference over channels, and the 90th and 99th percentiles of each pixel's
largest channel difference. SSIM: Wang et al. 2004 on display luma, Gaussian window σ 1.5.
Sky hue: the per-channel median of a patch of sky picked by ray direction (sRGB), and its
HSV hue, as the engine reports it: *band*, 2–4° up and 12–25° left of the setting sun; *upper*,
6–9° up at the same bearing; *away*, 5–9° up and 60–73° left of the sun (the frame's left).
Lumenwright's noise floor: the dusk frame against itself with another seed is display mean
1.45, p90 7, p99 14, SSIM 0.9327 [measured].

**Dusk** (tick 12800):

| frame | display mean | p90 | p99 | SSIM | mean luma | band | upper | away |
|---|---|---|---|---|---|---|---|---|
| Chunky, the engine's light (reference) | | | | | 54.2 | (178, 97, 98) 359° | (166, 96, 103) 354° | (151, 95, 110) 344° |
| main (`38ef002`): Chunky's sky and sun, `gamma` | 18.68 | 124 | 156 | 0.7799 | 73.5 | (249, 221, 123) 47° | (230, 217, 147) 51° | (147, 145, 115) 56° |
| main + the mid-grey tone curve | 14.25 | 90 | 106 | 0.7946 | 68.7 | (202, 188, 122) 50° | (193, 186, 141) 52° | (142, 140, 115) 56° |
| the game's sky, `gamma` | 9.72 | 40 | 126 | 0.8103 | 56.4 | (205, 94, 57) 15° | (186, 92, 66) 13° | (168, 92, 76) 10° |
| the game's sky and the mid-grey curve (`f291590`, the defaults) | 7.12 | 35 | 86 | 0.8125 | 56.0 | (180, 96, 60) 18° | (168, 95, 69) 16° | (157, 94, 78) 12° |

**Day** (tick 4000):

| frame | display mean | p90 | p99 | SSIM | mean luma | band | upper | away |
|---|---|---|---|---|---|---|---|---|
| Chunky, the engine's light (reference) | | | | | 105.8 | (213, 212, 204) 53° | (210, 210, 204) 60° | (212, 211, 204) 52° |
| main (`38ef002`): Chunky's sky and sun, `gamma` | 30.84 | 65 | 77 | 0.7267 | 113.4 | (154, 172, 185) 205° | (139, 172, 196) 205° | (147, 175, 195) 205° |
| main + the mid-grey tone curve | 33.17 | 68 | 75 | 0.7416 | 110.3 | (147, 159, 168) 206° | (135, 159, 174) 203° | (142, 161, 174) 204° |
| the game's sky, `gamma` | 9.67 | 51 | 51 | 0.9348 | 102.4 | (182, 208, 255) 219° | (167, 196, 255) 220° | (176, 203, 255) 219° |
| the game's sky and the mid-grey curve (`f291590`, the defaults) | 8.37 | 47 | 55 | 0.9321 | 98.3 | (166, 181, 204) 216° | (156, 175, 204) 216° | (162, 179, 204) 216° |

**The defaults at `0cd2b54`** (by the time of day, the fitted lanterns, no emitter flags;
CUDA, 16384 samples, four bounces), against the same references [measured]:

| frame | display mean | p90 | p99 | SSIM | mean luma | band | upper | away |
|---|---|---|---|---|---|---|---|---|
| dusk (day weight 0.181818, emitter intensity 115.2423) | 8.06 | 33 | 86 | 0.8162 | 59.6 | (190, 127, 77) 27° | (178, 125, 90) 24° | (156, 105, 87) 16° |
| day (Chunky's look, emitter intensity 13) | 31.46 | 65 | 79 | 0.7161 | 111.8 | (154, 172, 185) 205° | (139, 172, 196) 205° | (147, 175, 195) 205° |

By day the default is Chunky's look by ruling, not the engine's, so the day reference is a
record of the difference and not a target. At dusk the lantern-lit pixels read 41.993, as
the reference's (the fit); the frame's mean luma is above the reference's because Chunky's
sky enters at 0.18 and Lumenwright's lens shows the client's sky without the veil.

At the `gamma` step the reference is the same Chunky render re-shown with `GAMMA`: dusk display
mean 17.67 (main) and 7.78 (the game's sky), day 42.38 and 12.01.

* [measured] The two departures account for what is left in the sky. A build of `a86e929`
  with both turned back to the engine's choices (the lens's map carrying the veil, and the clear
  colour's weight `1 − clamp(x, 0.25, 1)^0.25`), on Metal at 256 samples, reads the sky patches
  at dusk (178, 99, 100) 359°, (166, 97, 105) 353°, (151, 96, 111) 344° and by day
  (213, 213, 204) 60°, (210, 210, 204) 60°, (211, 212, 204) 68°: the reference's to within
  three levels; its mean luma is 55.3 and 106.0 against 54.2 and 105.8, and by day its frame
  is display mean 3.38 from the reference. With the client's lean and no veil in the lens, the
  dusk sky is the orange of the client's sunset fan (hue 12–18°) where the reference is mauve
  (344–359°), and the day sky the client's blue (216°) where the reference is the veil's
  near-white.
* [measured] At dusk the light the sky casts, peak channel, is 0.368591 here (the lightmap
  at its 8-bit level) and 0.36634 in the engine's scene: 0.6%.
* [measured] The day frames' SSIM (0.93) is near the noise floor's; at dusk (0.81) the
  difference is the sky (the hues above) and the lanterns' noise under emitter sampling
  `NONE`, which Chunky's frame averages over ten paths per sample (`branchCount` 10).

## Not expressed

* Clouds, and the weather: Lumenwright's environment sampler applies no rain or thunder
  layer, so every frame is drawn clear.
* The dark disc the client draws under a camera below the world's horizon height.
* A face that sees open sky under the horizon (the side of a block over the void) takes light
  from the clear colour there, warm on the sun's side, which the game does not cast: the light
  is exact on an open top face (the engine's record says the same of its scenes).
* Quick mode's lightmap still takes its daylight from the pre-1.21.11 day curve; HQ mode's
  game sky takes it from the timeline's `sky_light_factor` and `sky_light_color`.
