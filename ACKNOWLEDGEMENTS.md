# Acknowledgements

Code, algorithms and constants Lumenwright takes from other works, with their licences.

## Chunky

* Source: <https://github.com/chunky-dev/chunky>, read at revision `156e2bba2526` (the source of
  Chunky 2.5.0-SNAPSHOT.474).
* Licence: GNU General Public License, version 3 or (at your option) any later version. The
  repository's `LICENSE` is the GPL-3.0 text, its README grants redistribution under the GPLv3,
  and the source files carry the "version 3 of the License, or (at your option) any later
  version" notice. Copyright (c) 2010-2023 Jesper Öqvist and Chunky contributors.
  Lumenwright is GPL-3.0-or-later, so the ported code is used under the same licence.
* Ported into HQ mode (each port names its Chunky file in a code comment; the full map is
  [docs/research/chunky-parity.md](docs/research/chunky-parity.md)):
  * the path tracer's diffuse event and ray depth (`PathTracer.java`);
  * the sun's emittance at every altitude, default intensity and cone sampling (`Sun.java`),
    the per-direction `frontLight` test (`PathTracer.java`), the `FAST` sun sampling strategy
    (`SunSamplingStrategy.java`), and the sun drawn in the sky with the client's texture
    (`Sun.intersect`, `Sky.addSunColor`, `Texture.getColor`);
  * the Preetham sky (`PreethamSky.java`), its constants and its evaluation, and the sky
    cache it is read through (`SkyCache.java`, `ColorUtil.RGBtoHSL` and `RGBfromHSL`);
  * the emitter model: emittance per block material (`BlockPalette.java`), quadratic emitted
    light, the default emitter intensity (`Scene.java`), the emitter grid (`Grid.java`), the
    emitter sampling strategies (`EmitterSamplingStrategy.java`) and face sampling
    (`PathTracer.sampleEmitter`, `sampleEmitterFace`);
  * the display gamma 2.2 for decoding textures (`ColorUtil.java`) and encoding frames, and
    the gamma, ACES and Hable post-processing filters (`renderer/postprocessing`);
  * one-sided block faces (`Quad.java`, `AABB.java`, `AABBModel.java`), cell bounds computed
    per step (`Octree.enterBlock`), the grass block's overlay over its side
    (`GrassBlockModel.java`), and the unknown biome's climate (`Biomes.java`).

## Delvewright's engine

* Source: <https://github.com/stellarfeline/delvewright>, read at revision `0fd300da7`.
* Licence: GNU General Public License, version 3. The repository's `LICENSE` is the GPL-3.0
  text; its README states GPL-3.0-or-later and its `Cargo.toml` `GPL-3.0-only`. The ported
  code is used under the GPL-3.0, which Lumenwright's GPL-3.0-or-later admits.
* Ported into HQ mode's game sky ([docs/research/game-sky.md](docs/research/game-sky.md); each
  port names its engine file in a code comment):
  * the Hable curve read brighter by the gain that shows mid-grey as the game's transfer does,
    in closed form (`HableCurve::brightened`, `mid_grey_gain`, `tone_curve` in
    `crates/delvec/src/compiler/view/scene.rs`), copied into `render/post.rs`;
  * the method that lights a scene as the 1.21.11 client lights a tick: no sunlight, the
    lightmap's colour for a cell open to the sky as the light (`lightmap_sky`, `sky_cell` in
    `scene.rs`, transcribed again from the client's `lightmap.fsh` in `render/sky.rs`), and
    the client's sky plus the least uniform veil, scaled, as that light's directions
    (`sky_map` in `crates/delvec/src/compiler/view/sky_image.rs`).

## The Preetham sky model

* A. J. Preetham, P. Shirley and B. Smits, "A Practical Analytic Model for Daylight",
  SIGGRAPH 1999. Chunky's `PreethamSky` implements it; Lumenwright ports Chunky's
  implementation, so the coefficients are the paper's as Chunky carries them.

## Numerical approximations

* `acos` and `atan2` in the kernel: M. Abramowitz and I. A. Stegun, *Handbook of
  Mathematical Functions*, formulas 4.4.46 and 4.4.49 (a public-domain US government
  publication).

## Twilight

* The end of civil twilight, the sun's centre 6 degrees below the horizon, at which HQ
  mode's Chunky sky (`--sky chunky`) reaches a night's light: the definition the US Naval
  Observatory publishes (a fact, not a work; no licence applies).

## Mid-grey

* Linear 0.18, the reflectance of the photographic grey card exposure is metered against
  (Kodak Gray Card, publication R-27), the value the game's tone curve is anchored at (a fact,
  not a work; no licence applies).
