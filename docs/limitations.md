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
* Entity layers drawn: the sheep's fleece and dyed undercoat, the eyes of spiders, cave
  spiders and endermen, the warden's bioluminescence, the happy ghast's harness and the
  camel's saddle. Not drawn: other second-texture layers (the drowned's, stray's and
  bogged's outer layers, the phantom's eyes, horse and llama equipment, the wither's
  armour, the happy ghast's ropes), and the warden's pulsating spots, heart and tendrils,
  whose glow is a quarter strength or nothing at tick 0. Glowing eyes and the glow of the
  allay, glow squid and wither light themselves in quick mode only; HQ mode draws them lit
  by the scene, as Chunky draws entities.
* Entities are posed as their models pose them at rest at tick 0 (the ghasts' tentacles
  at the middle of their swing): a parrot stands, an axolotl rests on the ground, a squid
  lies where it is placed. Babies draw their own model where vanilla bakes one that is not
  the adult halved (the ghastling, the baby turtle and camel); others are the adult at half
  scale. The bat is not drawn: its pose is a keyframe animation.
* A boat on water shows the water inside its hull; vanilla masks it with the boat's water
  patch. A minecart is turned by its stored yaw and pitch, which follow its rail; vanilla
  turns it by the rail itself, a few thousandths of a block off. A firework rocket is its
  item turned to the camera, with no trail.
* HQ mode's sea beyond the loaded chunks is Chunky's water plane, off by default
  (`--water-plane`, [docs/research/sea-horizon.md](research/sea-horizon.md)); without it a
  level frame over an ocean shows the loaded sea's edge below the horizon. HQ mode draws no
  render-distance fog.
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
* On the client's day plateau HQ mode's emitters are off unless `--emitter-intensity` states
  one, as in Chunky's default scene: a room lit only by lanterns is dark at noon
  ([docs/research/daylight-emitters.md](research/daylight-emitters.md)).
* The game's sky draws no clouds and no weather, and HQ mode's lanterns are Chunky's emitters,
  not the game's block light ([docs/research/game-sky.md](research/game-sky.md)).
