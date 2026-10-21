# Known limitations

[Lumenwright](../README.md)

## Known limitations (v0.1)

See [docs/SCOPE.md §3](SCOPE.md). The main gaps:
* Beds, shulker boxes and the conduit are not drawn. Chests are drawn closed and bells at
  rest; sign text is drawn without italic, underline or obfuscation.
* Items that vanilla draws in Java code are not drawn, the shield excepted: chests, beds,
  banners, heads, shulker boxes, decorated pots, the conduit and copper golem statues. They
  are counted in `items_skipped`. A filled map in a frame shows the map item, not its
  picture; armour trims and the elytra are not drawn.
* Entity layers that need a second texture are missing (sheep wool, glowing eyes).
* Smooth fluid surfaces, clouds, weather and biome blending are not implemented. In quick
  mode the End's sky box is drawn as its fog colour, the dark disc vanilla draws below the
  horizon for a camera under sea level is not drawn, and the lightmap's daylight still
  follows the pre-1.21.11 day curve.
* HQ mode lights emitters as Chunky does, and Chunky's emitter sampling is not a light
  transport estimate: it picks one emitter near the surface without dividing by the pick's
  probability, so a surface among many emitters gets the average of their light rather
  than the sum, and an emitter farther than 10 to 19 blocks along an axis (by where the grid
  cells fall) is never sampled; it lights a surface only through bounced paths that meet it.
* Not yet ported from Chunky: biome blending, candle and campfire flame particles, its
  fancier translucency, water shading, metals and its Nishita sky
  ([docs/research/chunky-parity.md](research/chunky-parity.md)).
* Chunky has no time of day. Under `--sky chunky` HQ mode keeps Chunky's sky while any part
  of the sun's disc is above the horizon; below it, the sky falls to a night's 3% at the end
  of civil twilight (the sun 6° below the horizon), and the sun gives no light from below the
  horizon. Those two rules are Lumenwright's own. The default game sky has none.
* The game's sky draws no clouds and no weather, and HQ mode's lanterns are Chunky's emitters,
  not the game's block light ([docs/research/game-sky.md](research/game-sky.md)).
