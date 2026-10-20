# Architecture

[Lumenwright](../README.md)

## Architecture

```
crates/lumenwright/        the library
  world/      Anvil regions, paletted chunk sections, light, biomes, entities, level.dat
  assets/     blockstate variants/multipart, model baking, atlas, biome tints, font, lang
  scene/      material-resolved quads; quick mesh (culling, smooth light + AO);
              HQ brick grid (16³ bricks, u16 model instances, entity overlay cells); entity models
  request.rs  the entry point: Request -> render() -> Rendered
  render/     Backend trait
    cpu/        tile-binned software rasterizer + lumenwright-kernel on rayon
    gpu/        wgpu: quick.wgsl (raster pipeline), pathtrace.wgsl (compute port of the kernel)
    cuda.rs     lumenwright-kernel compiled to PTX, launched via cudarc
crates/lumenwright-cli/    the `lumenwright` command, over the library's public API
  main.rs     subcommands, logging
  job.rs      render: flags -> Request -> render() -> PNG/PFM
  jar.rs      client-jar resolution and download
  output.rs   PNG and PFM writers
  values.rs   flag values for the library's option enums
  fixture.rs  generated worlds for demo-world and the tests
crates/lumenwright-kernel/ #![no_std] path tracer shared by CPU and CUDA (same source)
crates/lumenwright-ptx/    CUDA entry point (builds for nvptx64-nvidia-cuda)
```

Both modes share one scene build. Quick mode turns visible quads into a triangle mesh, the
way the vanilla client does. HQ mode instances block models inside a sparse two-level voxel
grid: empty bricks are skipped and fully buried bricks are collapsed, so a ray needs one
traversal loop and no BVH. Entities go into sparse overlay cells of the same grid.

Backend parity is by construction. The CPU and CUDA backends run the same Rust kernel. The
WGSL kernel mirrors it line by line and uses the same RNG, so CPU and Metal draw the same
random numbers for each pixel and sample. Builds are deterministic: identical inputs give
bit-identical images.

Quick mode samples block textures as the 1.21.11 client does: each sprite of the client's
block atlas (`atlases/blocks.json`) gets a mip chain built like `MipmapGenerator` builds it
(a linear-light 2×2 mean; cutout textures keep their alpha-test coverage; the `.mcmeta`
`mipmap_strategy` and `alpha_cutoff_bias` apply), padded and aligned like the client's
stitcher so no level bleeds into a neighbour, and read through `terrain.fsh`'s
`sampleNearest` with trilinear, 4× anisotropic filtering. Pixel art stays sharp up close and
distant terrain does not shimmer. Entities, items and text are read nearest, as the client
reads its other atlases. The quick sky is the client's `SkyRenderer`: the fogged sky disc, the
sunrise and sunset glow, the sun, this night's moon phase and the 1500 stars, with every
colour and angle read from the jar's environment attributes and day timeline (dimension,
biome at the camera, time of day). The world fog adds the client's environmental fog to the
render-distance fog. HQ mode's default sky is this sky, drawn into a map its path tracer
reads; both modes place the sun at the timeline's `sun_angle`.

Mob geometry lives in Java code in vanilla, not in assets, so it ships as data in
[`crates/lumenwright/assets/entity_models.json`](../crates/lumenwright/assets/entity_models.json):
57 mobs in vanilla's own part, cube and box-UV terms, transcribed from 1.21.11. Adding a mob
needs no Rust changes. Block displays, paintings and mannequin or player skins are built in
code. Anything else is drawn as a box the size of its hitbox.

Text displays are drawn in the jar's default font (`assets/minecraft/font/default.json` and
the bitmap providers it names), placed as the client places them: the `text` component with
its colours, bold, italic and `translate` keys, split into lines no wider than `line_width`,
`alignment`, the `background` colour or `default_background`, `shadow`, `billboard` and
`transformation`. A `see_through` display is drawn over whatever stands in front of it. A text
display is seen from the front only, as in the game. `text_opacity` and `brightness` are not
read. A character no bitmap provider covers is drawn from the Unifont (see
[Textures](textures.md#textures-the-chunky-approach)), cut and spaced as the client's unihex provider cuts
and spaces it; without a Unifont directory it is left out.

![entities](renders/entity-lineup.png)

Block entities that vanilla draws in Java code use the same format, in
[`crates/lumenwright/assets/block_entity_models.json`](../crates/lumenwright/assets/block_entity_models.json),
each model naming the 1.21.11 client class it is transcribed from. Their data comes from the
chunk's `block_entities`:

* signs and hanging signs, with front and back text in the vanilla bitmap font
  (`assets/minecraft/font`), dye colour, glowing text and its outline. A `translate` line
  resolves through the `en_us` language files of the jar and the resource packs;
* bells, the lectern's book and the enchanting table's book;
* chests: single, double, trapped, ender and copper;
* banners, with their pattern layers composited in their dye colours;
* decorated pots with their sherds;
* skulls and heads. A player head's skin is the profile's `texture` asset when a resource
  pack provides it, otherwise the default skin for the profile's UUID. Skins are never
  downloaded.

![block entities](renders/block-entities-quick-metal.png)
![signs](renders/block-entities-signs.png)

Items are drawn through the jar's item model definitions (`assets/minecraft/items/*.json`,
the format of 1.21.4 and later), resolved against the stack's components and where it is
drawn. A model with elements is baked as a block model; a flat item is extruded from its
sprite, a front and a back face and one face for every open texel edge, as the client does.
The model's display transform for its context poses it:

* dropped items (`ground`), bobbing and turned by a phase taken from the entity's UUID, as
  a cluster of up to five copies for a large stack;
* item frame and glow item frame contents (`fixed`), turned by `ItemRotation`;
* item displays, in their `item_display` context, `transformation` and `billboard`;
* items in the hands (`thirdperson_righthand`, `thirdperson_lefthand`) and non-armour items
  on the head (`head`) of armour stands, mannequins, players and humanoid mobs. Armour
  stands take their `Pose` and `ShowArms`.

Worn armour is drawn from the jar's equipment assets (`assets/minecraft/equipment/*.json`)
on the humanoid armour meshes, dyed leather included. The shield is drawn from data in
[`crates/lumenwright/assets/item_models.json`](../crates/lumenwright/assets/item_models.json),
with the armour meshes, each citing its 1.21.11 client class. `EntityStats` counts the
stacks drawn and not drawn by item id (`items_rendered`, `items_skipped`).

![items](renders/items-quick.png)

```sh
target/release/lumenwright demo-world --items /tmp/items
target/release/lumenwright render -w /tmp/items \
    --camera 7.5,67.6,9.5 --look-at 7.5,66.0,2.5 --fov 60
```
