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
  * the sun's emittance, default intensity and cone sampling (`Sun.java`), the `FAST` sun
    sampling strategy (`SunSamplingStrategy.java`);
  * the Preetham sky (`PreethamSky.java`), its constants and its evaluation;
  * the emitter model: emittance per block material (`BlockPalette.java`), quadratic emitted
    light, the default emitter intensity (`Scene.java`), the emitter grid (`Grid.java`), the
    emitter sampling strategies (`EmitterSamplingStrategy.java`) and face sampling
    (`PathTracer.sampleEmitter`, `sampleEmitterFace`);
  * the display gamma 2.2 for decoding textures (`ColorUtil.java`) and encoding frames, and
    the gamma, ACES and Hable post-processing filters (`renderer/postprocessing`);
  * one-sided block faces (`Quad.java`, `AABB.java`, `AABBModel.java`), cell bounds computed
    per step (`Octree.enterBlock`), the grass block's overlay over its side
    (`GrassBlockModel.java`), and the unknown biome's climate (`Biomes.java`).

## The Preetham sky model

* A. J. Preetham, P. Shirley and B. Smits, "A Practical Analytic Model for Daylight",
  SIGGRAPH 1999. Chunky's `PreethamSky` implements it; Lumenwright ports Chunky's
  implementation, so the coefficients are the paper's as Chunky carries them.

## Numerical approximations

* `acos` in the kernel: M. Abramowitz and I. A. Stegun, *Handbook of Mathematical Functions*,
  formula 4.4.46 (a public-domain US government publication).
