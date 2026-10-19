# Emitters in HQ mode: research and the rule chosen

Question: how should a path-traced Minecraft renderer make an emissive block look right up close
and also light the room around it the way the game shows it?

Each statement below is marked **[cited]** (the source says it) or **[inferred]** (our reading
of what a source does). Sources that are not freely licensed are used for ideas only.

## What the game does

* [cited, the pinned client and `lumenwright::scene::mesh::lightmap`] Block light is an integer
  level 0–15 per block. It falls by one level per block of Manhattan distance from the source
  (a torch is 14, glowstone 15). The lightmap turns a level `l` into a brightness through
  `f / (4 − 3f)` with `f = l / 15`, then a gamma adjustment. Light from a surface's angle to the
  source plays no part.
* [inferred] Between one and seven blocks from a torch (levels 13 and 7) the curve falls to about
  0.29 of its value. Inverse-square falloff over the same range falls to 1/49 ≈ 0.02, roughly
  14 times faster. A physically lit room therefore looks dark beyond a few blocks next to the
  game.

## Chunky (GPL-3.0; read at `f10ef3fab4f0`)

* [cited] `Scene.java`: the default emitter intensity is 13 (allowed range 0.01–1000).
  Emitters are off by default, and the default emitter sampling strategy is `NONE`.
* [cited] `BlockPalette.java`: emittance is one number per material. It is mostly 1.0 (torch,
  glowstone, lantern, lava, fire). Soul torch and crying obsidian are 0.6 and nether portal
  0.4. Only a few blocks tie it to their light level (sea pickle, respawn anchor).
* [cited] `PathTracer.java`: a texel emits `colour² × emittance × emitter intensity`. The source
  comment says: "Quadratic emittance mapping, so a pixel that's 50% darker will emit only 25% as
  much light. This is arbitrary but gives pretty good results in most cases."
* [cited] `EmitterSamplingStrategy.java`: four strategies. `NONE` samples no emitters, `ONE` a
  single face, `ONE_BLOCK` all faces of one block and `ALL` every face of every block. A grid of
  emitters (`Grid.java`, default cell size 10) limits the candidates to nearby cells.
* [inferred] `sampleEmitterFace` samples by area and picks an emitter uniformly. There is no MIS:
  an indirect ray that hits an emitter is ignored while sampling is on
  (`preventNormalEmitterWithSampling`).
* [cited] Open pull request chunky-dev/chunky#1627 notes that "the entirety of the blocks are
  emitting light despite the fact that physically, only the 'flame' portions should be providing
  light". It proposes emitter mapping by reference colours or by HSV value raised to an exponent
  (default 1.5).
* [cited] Closed pull request #1555 proposed a separate "apparent emitter brightness" and
  "emitter light intensity". Merged #1621 linked the two again.
* [inferred] Chunky's maintainers chose to keep the look and the light physically coupled.

## Shader packs (ideas only: proprietary or unlicensed)

* [cited] Complementary Reimagined (its own licence; read at `c09950df065`) lights the room from
  the vanilla lightmap, shaped by a curve and scaled by "Blocklight Intensity" (`XLIGHT_I`).
* [cited] Complementary draws a surface's own glow as a separate term, scaled by
  `CUSTOM_EMISSION_INTENSITY` ("Adjusts the intensity of glowing surfaces").
* [cited] Kappa (a public fork, no licence) takes block light from the lightmap.
* [cited] Kappa adds screen-space path-traced emitter hits, windowed to zero by
  `ssptEmissionDistance` (default 8), and blends them with the lightmap (`ssptLightmapBlend`).
* [inferred] Both packs set the light a block casts and the brightness of its face with separate
  controls. Neither trusts inverse-square falloff alone for block light.
* SEUS PTGI and BSL: no primary source was found. Not established.
* [cited] LabPBR (shaderlabs.org wiki, CC BY-NC-SA, so we take the idea only) stores per-texel
  emission, linear 0–254, in the alpha channel of the specular texture.

## Light sampling in the literature

* [cited] PBRT 3e §13.10: "the power heuristic often reduces variance even further" than the
  balance heuristic, and β = 2 is the empirical choice (after Veach 1997). Light sampling is the
  better strategy for "diffuse or glossy BRDFs and small light sources".
* [cited] PBRT 4e, light sampling: "Sampling lights based on their power usually works well",
  though it ignores where the lights are. A light BVH (Conty Estevez & Kulla 2018, DOI
  10.1145/3233305) or ReSTIR (Bitterli et al. 2020) handles hundreds of thousands of lights.

## Making the look and the light differ

* [cited] Blender's Light Path node gives "Is Camera Ray". Plugged into emission strength, it
  makes an emitter look one way to the camera and light the scene another way (Cycles4D manual).
* [cited] Unreal Lightmass `EmissiveBoost` "scales the emissive contribution of all materials
  applied to this object" to baked lighting, apart from what the camera sees.
* [cited, blog-level sources] In cinematography, practical lights in frame are normally allowed
  to clip while the subject is exposed correctly.
* [inferred] A clipped flame is normal. A whole block face clipped to white is a fault of how
  texels are mapped to emission, not of exposure.
* [cited] Karis 2013 ("Real Shading in Unreal Engine 4"): windowed inverse-square falloff,
  `saturate(1 − (d/r)⁴)² / (d² + 1)`.
* [cited] Unreal also offers a non-physical falloff exponent for fill lights.
* [inferred] A windowed or non-physical falloff would change what light sampling estimates.
  BSDF hits on the emitter would then no longer agree with it, so MIS would be inconsistent.

No source was found that calibrates a path tracer to Minecraft's 15-level linear falloff. The
calibration rule below is therefore **authored**, not cited.

## The rule chosen

1. **Next-event estimation with MIS** [cited practice: PBRT, Veach].
   * Each diffuse vertex samples one point on one light-casting quad instance. The quad is
     picked with probability proportional to its power.
   * The point is uniform over the quad's area.
   * Its weight against cosine BSDF sampling uses the power heuristic, β = 2.
   * The kernel, the WGSL mirror and the CUDA build share one buffer layout and one random
     stream.
   * Paths that crossed water since their last diffuse bounce leave emitter light to light
     sampling alone, as the sun already does.
2. **The look and the light are separate** [cited practice: Blender "Is Camera Ray", Unreal
   `EmissiveBoost`, Complementary; departs from Chunky #1621].
   * A camera ray, or a chain of specular events from it, sees `texel × level/15 × glow`
     (`--emitter-glow`, default 1.5).
   * A diffuse surface receives the quad's emitted light.
3. **Texels emit by the square of their colour** [ported from Chunky's quadratic emitter
   mapping, GPL-3.0]. A torch's bright flame lights the room; its dark stick barely does.
4. **A block casts power by its light level, not by its shape** [authored].
   * Each block of light level `L` casts total power `P15 × (L/15)²`, shared over its glowing
     quads by area × mean squared texel luminance.
   * The reason for the square: under inverse-square falloff, the distance at which irradiance
     falls to a given value grows with the square root of the power. Power ∝ L² therefore makes
     a block's reach proportional to its level, as one level per block does in the game.
   * A torch and a glowstone block then light a room about equally, as they do in the game,
     although the torch's glowing area is a twentieth of the block's.
   * `--emitter-intensity` (default 1) scales all of it.
   * `P15` is calibrated by the `emitter_calibration` test: a torch on a white floor, compared
     with the quick render level by level.
5. Falloff stays inverse-square, so the image is a physical light transport of the calibrated
   powers. The calibration test states how far that falls from the game's straight line.

Not adopted: a windowed falloff (it breaks MIS consistency), and a spatial light structure (light
BVH, Chunky's grid, ReSTIR). With many emitters in view, power-proportional picking spends most
samples on lights far from the shading point. That cost is open work.
