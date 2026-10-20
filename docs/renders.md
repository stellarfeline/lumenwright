# The Treehouse Camp renders

[Lumenwright](../README.md)

## How they are drawn

HQ mode runs with its defaults: the game's sky at the hour, lit as the 1.21.11 client lights
it (the colour its lightmap gives a cell open to the sky, from the sky's directions, and no
sunlight, so nothing casts a sun shadow), the client's sky, sun, moon and stars in view,
lanterns and campfires as Chunky's emitters at intensity 13 with one emitter sampled per
diffuse bounce, and the tone curve that shows mid-grey as the game does.

At `--time 12800` the sun stands just below the horizon. Quick mode draws the client's sunset:
the sun, the orange glow and the light the game stored in the save. HQ mode draws the same sky
and lights the camp with the evening's dim blue light; its lanterns are Chunky's emitters,
which light little more than the block they stand on, where the game's block light reaches
14 blocks (see [docs/research/game-sky.md](research/game-sky.md)).

Each image's command, run from the repository root with the save from
[Get the camp](#get-the-camp) in `the-treehouse-camp`. The HQ commands need the CUDA build
(see [Backends](backends.md#backends)); on another backend the same flags give the same image within the
parity the tests hold (`gpu_matches_cpu_in_hq_mode`, `cuda_matches_cpu_in_hq_mode`).

```sh
# The whole camp
target/release/lumenwright render -w the-treehouse-camp --camera 140,118,14 --look-at 56,82,58 --fov 60 --render-distance 12 \
    --time 4000 --backend metal -o treehouse-camp-day-quick.png
target/release/lumenwright render -w the-treehouse-camp --camera 140,118,14 --look-at 56,82,58 --fov 60 --render-distance 12 \
    --time 4000 --backend cuda --mode hq --spp 4096 --max-bounces 8 -o treehouse-camp-day-hq.png
target/release/lumenwright render -w the-treehouse-camp --camera 140,118,14 --look-at 56,82,58 --fov 60 --render-distance 12 \
    --time 12800 --backend metal -o treehouse-camp-dusk-quick.png
target/release/lumenwright render -w the-treehouse-camp --camera 140,118,14 --look-at 56,82,58 --fov 60 --render-distance 12 \
    --time 12800 --backend cuda --mode hq --spp 16384 --max-bounces 8 -o treehouse-camp-dusk-hq.png

# The Long Bridge
target/release/lumenwright render -w the-treehouse-camp --camera 60,93,22 --look-at 60,80,42 --fov 70 \
    --time 4000 --backend metal -o treehouse-bridge-day-quick.png
target/release/lumenwright render -w the-treehouse-camp --camera 60,93,22 --look-at 60,80,42 --fov 70 \
    --time 4000 --backend cuda --mode hq --spp 4096 --max-bounces 8 -o treehouse-bridge-day-hq.png
target/release/lumenwright render -w the-treehouse-camp --camera 60,93,22 --look-at 60,80,42 --fov 70 \
    --time 12800 --backend metal -o treehouse-bridge-dusk-quick.png
target/release/lumenwright render -w the-treehouse-camp --camera 60,93,22 --look-at 60,80,42 --fov 70 \
    --time 12800 --backend cuda --mode hq --spp 16384 --max-bounces 8 -o treehouse-bridge-dusk-hq.png
```

| image | Apple M2 Pro, Metal (end to end) | RTX 4070 Ti SUPER, CUDA (end to end / in the kernel) |
|---|---|---|
| camp, quick, day / dusk | 0.27 s / 0.27 s | |
| camp, hq, day (4096 spp) / dusk (16384 spp) | | 92 s / 333 s (90 s / 331 s) |
| bridge, quick, day / dusk | 0.23 s / 0.23 s | |
| bridge, hq, day (4096 spp) / dusk (16384 spp) | | 123 s / 447 s (121 s / 444 s) |

The quick times are the second of two runs. The 4070 Ti SUPER runs under WSL2 on a 16-thread PC.
Each HQ image rendered twice with the same command on the CUDA backend gave byte-identical
PNGs, as each quick image did on Metal.

### Get the camp


The camp ships as a container image that runs the map's own server. The server builds the
camp over its first ticks. These commands wait for it to finish, then save the world and copy
it out. They need Docker. `EULA=TRUE` accepts [Mojang's EULA](https://aka.ms/MinecraftEULA)
for the server inside the container.

```sh
docker run -d --name treehouse-camp -e EULA=TRUE ghcr.io/stellarfeline/delve-the-treehouse-camp:v1.0.1
# The camp is stamped by its datapack over the first ticks; wait until it says so.
until docker exec treehouse-camp rcon-cli scoreboard players get '#placed' dw.sys 2>/dev/null | grep -q 'has 1'; do sleep 5; done
docker exec treehouse-camp rcon-cli save-all flush
docker stop treehouse-camp
docker cp treehouse-camp:/data/world the-treehouse-camp
docker rm treehouse-camp
```

The image is release v1.0.1 of the camp, built by `delvec` 1.12.0 from
`campaigns/the-treehouse-camp/` in
[stellarfeline/delvewright-campaigns](https://github.com/stellarfeline/delvewright-campaigns)
at the release's tag. The campaign's `GENERATION.md` there records how it was built and
checked. The save is written by a running server, so it is not byte-identical from one run
to the next: the same frame of two saves from two server runs differed in 257 of its 921,600 pixels.

### Render


```sh
# Inspect a save
target/release/lumenwright info the-treehouse-camp --entities

# List the block entities (and sign text) in a box of x,z block coordinates
target/release/lumenwright info the-treehouse-camp --block-entities=0,0,112,112

# Vanilla-look render from the save's spawn, at the save's own time of day
target/release/lumenwright render -w the-treehouse-camp -o quick.png

# Path-traced render from an explicit camera, under the game's sky at the hour
target/release/lumenwright render -w the-treehouse-camp --mode hq --spp 256 \
    --camera 140,118,14 --look-at 56,82,58 --time 12800 -o hq.png

# The same under Chunky's sky, with a sun that lights and shadows
target/release/lumenwright render -w the-treehouse-camp --mode hq --spp 256 --sky chunky \
    --camera 140,118,14 --look-at 56,82,58 --sun-altitude 35 --sun-azimuth 120 -o hq-chunky.png

# Generated worlds, no download: a small showcase (terrain, cottage, lake, mobs)
target/release/lumenwright demo-world showcase
target/release/lumenwright render -w showcase --mode hq -o showcase.png

# One of every block entity Lumenwright draws (docs/renders/block-entities-quick-metal.png)
target/release/lumenwright demo-world --block-entities gallery
target/release/lumenwright render -w gallery \
    --camera 9.5,70.5,11.5 --look-at 9.5,65.4,-2 --fov 60 -o gallery.png
```

`lumenwright backends` lists the backends available on the current machine.
