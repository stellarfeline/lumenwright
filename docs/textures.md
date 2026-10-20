# Textures

[Lumenwright](../README.md)

## Textures: the Chunky approach

Mojang's assets are not redistributable, so Lumenwright ships none. Like Chunky, it reads
textures, blockstates and block models straight out of the official client jar. The jar is
resolved in this order:

1. `--minecraft-jar <path>`
2. A launcher's copy for the world's version: the vanilla launcher's
   `versions/<v>/<v>.jar`, Prism Launcher's library, or Lumenwright's own cache
3. Chunky's `~/.chunky/resources/minecraft.jar`, if its version matches
4. A download from Mojang's version manifest (`piston-meta.mojang.com`), SHA-1 verified, into
   the cache (`~/Library/Caches/lumenwright` or `$XDG_CACHE_HOME/lumenwright`). `--offline`
   disables this; `lumenwright assets fetch <version|latest>` downloads ahead of time.

This resolution and the downloader belong to the command. The library takes the jar as a path.

The jar's own `font/include/unifont.json` lists no providers: the client reads the Unifont that
draws CJK and every other character no bitmap font covers from the launcher's asset index. The
command fetches it the same way, in this order: `--unifont <dir>` (a directory holding
`minecraft/font/include/unifont.json` and the hex zip it names, laid out like the index's keys),
then the cache (`<cache>/unifont/<version>`, every file checked against the SHA-1 its lock
records), then a download through the version's asset index, the index and every file checked by
SHA-1 and size. `--offline` forbids the download and refuses, naming what is missing, when
neither of the first two has the files. `lumenwright assets fetch-unifont <version|latest>`
downloads ahead of time. The library takes the directory as `Request::unifont` and does the same
checks on it without any network.

`--resource-pack` (zip or directory, repeatable) layers packs on top of the vanilla assets.
