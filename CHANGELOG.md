# Render Mill changelog

Release notes, newest first. The app was called LumenCraft up to 0.13.0; commands in older notes use the old `lumencraft` name (now `RenderMill`).

## 0.16 — COLLADA (.dae) models

Render Mill opens COLLADA `.dae` files: Import model, Start from a Revit / 3D
model, or drag the file onto the window. This is the format Lumion LiveSync
for Revit, SketchUp, ArchiCAD and 3ds Max export.

- Units and up axis come from the file, so models arrive at the right size and
  the right way up (Units and Up axis in the import dialog can override them).
- Node hierarchies, instanced parts and per-object material assignments.
- Colours, textures, transparency (glass) and shininess. For files from
  Lumion's Revit exporter, its extra settings are used too: albedo colour and
  map, normal map, roughness, reflectiveness and material type.
- Textures are found from the paths in the file, or by file name next to the
  .dae (for example the `<model>_Textures` folder) if it has been moved.
- Like FBX, a `.dae` imported as the building is watched: export again and the
  scene updates.

## 0.15 — open Lumion files

Render Mill opens Lumion's own files (File › Open project, the start screen's
**Load**, or drag the file onto the window):

- **Lumion scenes (.lsf, and Lumion's quick-save .lqf):** your imported
  models where you placed them, the Lumion material settings on them (colour
  tints, roughness, metalness, transparency, and textures you added yourself),
  every saved camera view (Photo mode views and the first key of each movie
  clip), and the sun position.
- **Lumion's library objects** (trees, plants, people, cars, furniture,
  lights) are Lumion's licensed content, so they aren't copied: each one is
  replaced by Render Mill's nearest object at the same spot, size and
  rotation. When Lumion is installed on the same computer, Render Mill reads
  the names of its catalogue folders to tell a tree from a plant or a car.
  Grass clumps, landscape paint, text and effects are left out (Render Mill
  draws its own), and the activity log lists what wasn't brought over.
- **Lumion library materials** (Wood siding 003, Concrete 010...) are swapped
  for the closest Render Mill library material by name and kind.
- **Imported models from Lumion's library folder (.lib)**, e.g.
  `Documents\Lumion 2026\Library\Kapp.lib`, open like any other model
  (Import model or Start from a model).

The file format isn't published, so it was worked out from your own files;
everything is read, nothing is changed. Large scenes load in a few seconds.

## 0.14 — match materials from a photo

**Match from photo…** (Materials tab, or File menu) takes a photo of a house
and finds its materials in your library:

1. Pick a photo (JPG, PNG, WebP, TIFF...). Render Mill splits it into the
   materials it sees - siding, brick, roof, trim, paving - and tints each
   region in the preview (sky and plants are recognised and left out).
2. For every region it lists the closest textures from the built-in library
   and your own texture folders, with a similarity score. Click the photo or
   the list to pick a region, click a texture to choose a different match.
3. Each region is pointed at the model material whose name fits (a "Siding"
   match goes to your Revit "Siding - Lap" material, for example); change it
   in **Apply to**. **Apply** or **Apply all** puts the textures on the model.
   Undo works as usual.

It runs on this computer with image analysis of colour and texture (edge
strength, line direction such as siding courses, fine-scale variation) - no
cloud service, nothing uploaded. The first run "learns" every library texture
once (a few seconds for the built-in library, longer for big folders) and
caches it. Matches depend on the photo and on what is in your library:
treat them as suggestions and check them. You can also start Render Mill with
a photo (`RenderMill.exe house.jpg`), or test from a terminal with
`RenderMill.exe --match-photo house.jpg <texture folder> [overlay.png]`.

## 0.13.1 — LumenCraft is now Render Mill

The app is renamed **Render Mill**: the title bar, start screen, window name
and messages say Render Mill, the program is `RenderMill.exe` and the LiveSync
command-line tool is `rendermill-sync.exe`. Nothing else changes:

- Projects are still `.lcproj` files and open as before.
- Your settings (library folders, graphics card, recent projects, texture
  budget) are carried over from LumenCraft automatically on first start; they
  are now stored under `%APPDATA%\Render Mill`.
- The LiveSync protocol is unchanged, so BimCraft and existing sync clients
  keep working.
- Environment variables keep their `LUMENCRAFT_` names.
- The install folder can stay where it is; the old `lumencraft.exe` can be
  deleted once `RenderMill.exe` works.

## 0.13 — start screen, landscape templates, smoother camera

### Start screen and landscape templates
Render Mill now opens on a start screen like Lumion's: **Create new**,
**Example** (the demo house), **Load**, **Start from a Revit / 3D model**, and
your recent projects. **Create new** shows nine landscape templates: Green
meadow, Mountain valley, Lakeside, Forest clearing, Coast, Desert, Winter,
Golden hour and Flat lawn. Each one is real terrain (rolling hills and
foothills that run into the mountain backdrop, lakes or sea), thousands of
trees, shrubs, rocks, wildflowers and tall grass, and its own sky, sun and
haze. The building plot in the middle is always flat and clear. File › New
project opens the same page. Objects you place sit on the terrain.

### Import model dialog
Importing a model now asks how it should come in: as the **building**
(LiveSync, with units, up axis and "place on the plot") or as a **library
object** with your own name and category folder, then you click to place it.

### Smoother navigation
WASD/QE movement eases in and out and the mouse wheel glides instead of
jumping. Run **Measure-Viewport.bat** to record viewport frame times on your
card (`viewport-benchmark.txt`).

### Compatibility
Projects from 0.12 open unchanged (flat ground). New optional
`environment.landscape` in projects; settings.json gains `recent_projects` and
`model_labels`.

## 0.12 — GPU limits, texture budget, smoother live preview

### Photo and movie sizes checked against your graphics card
Before a render starts, Render Mill checks the chosen size against the card's
limits (largest texture, largest buffer, largest storage binding) and the
buffers the path tracer needs at that size. If it can't fit, the render popup
says why (for example "the sample buffer would need 265 MB; this card allows
128 MB per buffer") and offers a one-click **Use W x H (highest supported)**
button. A card that runs out of memory while allocating is caught the same way
instead of crashing. `--render` on the command line refuses with the same
message.

### Texture budget for large projects
The texture memory budget is now a hard limit for both the viewport and ray
tracing. When a project's maps don't fit, the largest are stored at lower
resolution first, down to 128 px (previously 512 px was the floor and the
budget could be exceeded). Only if even that isn't enough are normal/roughness
maps left out, then colour maps. Whenever quality is reduced you see it: a note
at the bottom of the viewport, a line in the photo render popup, the activity
log, and **Settings › Texture memory**, which shows usage and has a budget
slider (automatic = 2 GB viewport, 3 GB ray tracing).

### Fewer live-preview pauses
The ray-traced live preview no longer waits for the GPU read-back or the
denoiser: the image is resolved on the GPU, read back asynchronously and
denoised on a worker thread while the previous image stays on screen. Only
tone mapping runs when a new image arrives. Run **Measure-Preview.bat** (next
to RenderMill.exe) to record refresh times on your card into
`preview-benchmark.txt`; `RenderMill.exe --bench-preview [scene]` does the same
from a terminal.

### Compatibility
No project-file changes. New optional `texture_budget_mb` in settings.json.
Public API additions: `pathtrace::{check_resolution, max_resolution,
ResolutionLimit, TexturePlan}`, `PathTracer::{try_begin, request_preview,
poll_preview}`, `Renderer::{texture_stats, set_texture_budget,
render_post_only}`. Changed: `pathtrace::plan_classes` takes a data-map flag
list and returns a `TexturePlan`; `CLASS_SIZES` now has six classes
(128 px – 4096 px); `PtMemory` has `dropped`, `smallest_reduced` and `budget`.

## 0.11 — rendering quality, performance and stability

Full engineering notes, test results and benchmarks: `docs/v0.11-engineering-report.md`.

### More accurate, more realistic ray tracing
- **No more missing geometry:** the ray tracer's acceleration structure is now depth-bounded by construction, so its traversal can never silently skip parts of a large model. Tiny details (trim, mullions, hardware) are no longer dropped, and a single corrupt vertex can no longer hide unrelated geometry.
- **Brighter, correct interiors:** passing through glass no longer uses up the light-bounce budget (rooms lit through double-sided panes came out too dark), and presets trace deeper (Fast 4, High 8, Ultra 10, Max 14 bounces). White rooms lost ~18% of their light at the old depth.
- *Experimental:* window sampling (`LUMENCRAFT_WINDOW_SAMPLING=1`) aims sky light through glazing: ~43% less noise per sample indoors with identical brightness (tested), but each sample costs about twice as much, so it is off by default.
- **Sunlight through glass is tinted and reduced by reflection** like real glazing; before, windows cast no shadow at all.
- **Cleaner highlights:** the sun is no longer counted twice on glossy floors, worktops and water; rough metals and stone keep their brightness (multi-scatter energy compensation); sky light is consistent between what you see and what lights the scene.
- **Survey-coordinate models** (far from the origin) no longer get speckled self-shadowing.

### Sharper materials
- **Textures keep their resolution in ray-traced images:** 2K and 4K brick, stone, siding, roofing and wood stay 2K/4K. Before, loading many materials dropped *every* texture to 512 or 256 px. A memory budget (3 GB by default, `LUMENCRAFT_PT_TEXTURE_MB` to change) only steps down the very largest maps if needed.
- **Correct mipmaps** (in both the viewport and ray tracing): colour maps are filtered in linear light, so brick mortar, shingles and paving joints keep their contrast at a distance; normal maps stay crisp.

### Faster and smoother on RTX / eGPU
- Ray-traced work is queued asynchronously: the app no longer waits for the GPU after every pass, uses GPU timestamps (where supported) to size the work, and keeps passes short so the viewport stays responsive over Thunderbolt.
- Half the GPU→CPU data per preview update (fp16 read-back straight into the denoiser and back), pooled buffers, and the AI denoiser keeps its network loaded between preview updates.
- Changing a material or moving an object no longer rebuilds everything: geometry, textures and materials are cached separately.
- Lower memory use while loading big models (texture uploads are flushed as they go).

### Sturdier imports
- Damaged or truncated FBX files now show an error instead of crashing or freezing the app (offset/size/nesting checks, decompression limits). Exact duplicate meshes in exports are removed.

### For developers
- `lumencraft --bench-pt <small-exterior|large-exterior|interior|large-model|all>` — reproducible path-tracing benchmark (time, samples/s, GPU memory).
- New tests: BVH stress tests, GPU-vs-CPU traversal comparison, texture-detail, window-sampling unbiasedness, malformed FBX, offline WGSL validation. `cargo fmt --check` and `cargo clippy --all-targets` are clean.

## 0.10

- **Textures on imported buildings now show at their real size.** Materials from the library (and your own texture folders) are placed in metres on every face — "real-world mapping" — instead of relying on the model's UV coordinates. Revit exports often have UVs that are missing, stretched 0–1 across a whole wall, or in feet/inches, which made textures look smeared, giant, tiny or like a flat colour.
- Material editor › Placement › **Real-world mapping** turns it on or off per material (off = use the model's own UVs, e.g. for a model that was carefully UV-mapped).
- The ray tracer places textures exactly the same way.
- Projects saved with earlier versions switch their library textures to real-world mapping when opened.

## 0.9

### Faster on your hardware
- **Choose the graphics card** (Settings › Graphics card): pick your eGPU or the laptop GPU by name; Render Mill remembers it and uses it on the next start. Command-line renders use the same choice (or `LUMENCRAFT_GPU=3090` for one run).
- On a dedicated graphics card the viewport now starts at full quality (4 of 4).
- **Ray tracing is faster (about 2× in testing, more on fast cards):** each GPU dispatch traces many samples per pixel at once (tuned automatically to the card's speed, short enough to never trip the Windows GPU timeout — important over Thunderbolt), and the CPU no longer waits on the GPU between tiles.
- **AI denoiser on the GPU:** Open Image Denoise now keeps one device for the session, so it runs on your NVIDIA card through CUDA without start-up cost each time (the log says which device it uses).

### More photo-realistic
- **Ray-traced preview** in Photo mode (the camera button next to the menu under the preview): while the camera is still, the preview refines itself with path tracing and AI denoising — 1, 2, 4, 8 … samples — so you see the final photo before rendering. Move the camera and it drops back to the real-time view.
- **Filmic tone curve (AgX)**, on by default (Photo › Color correction): bright skies and sunlit walls roll off to white like a real camera instead of shifting hue; colours stay natural. Turn it off for the previous ACES look.
- **Real lens depth of field** in ray-traced photos: with the Depth of field effect on, the path tracer uses a thin-lens camera (round bokeh, correct blur through glass and foliage) focused on the set distance or whatever is in the centre of the frame.
- Softer, more natural 3D grass in ray-traced images.

## 0.8

### Ray-traced photos with an AI denoiser

Photo mode › Render (camera button) › **Ray tracing**: Off / Fast / High / Ultra / Max.

- A GPU path tracer (compute shader, BVH acceleration) renders the photo with physically based light transport: global illumination (light bouncing between walls, floors and ground), soft sun shadows, true mirror and glossy reflections, refraction and absorption in water, light through thin glass, translucent leaves, alpha cut-outs (leaves, fences), emissive materials and every placed light.
- Same materials, textures, sky, sun, fog and photo effects as the rasterizer, so ray tracing is a switch, not a different look. Exposure, bloom, depth of field, lens effects and colour grading are applied afterwards as usual.
- **AI denoiser:** the remaining grain is removed with Intel Open Image Denoise, a neural network trained on path-traced images, guided by albedo and normal layers so texture detail and edges stay sharp. Runs on the CPU (or CUDA on NVIDIA). If the `denoiser` folder is missing, an edge-aware wavelet filter is used instead.
- Fast = 16 samples/pixel, High = 64, Ultra = 256, Max = 1024. A progress card shows samples and time left; Cancel at any time. Photo sets render one after another.
- Movie mode › Render › Ray tracing renders every frame path-traced (with a quarter of the samples per frame, denoised).
- Command line: `lumencraft --render model.fbx out.png --raytrace high` (`fast|high|ultra|max|<samples>`, `--no-denoise`).

Ray tracing runs on any GPU through wgpu (DirectX 12 / Vulkan); it doesn't need RTX cores but is much faster on a recent card.

## 0.7

### Real 3D models in the object library

- **Built-in objects.** About 70 photo-scanned CC0 models from Poly Haven ship with the app:
  - Trees and plants (potted plants, shrubs, ferns, flowers)
  - Rocks
  - Sofas and chairs
  - Tables
  - Beds and cabinets
  - Lamps, chandeliers and street lamps
  - Vases and decor

  They have full colour, normal and roughness maps. Heavy scans were reduced for real-time use: trees to at most 90k triangles, everything else to 20–30k.
- **Where to find them.** They appear alongside the built-in objects in Nature, Interior and Exterior, with hover previews, tags and favourites.
- **Your own models.** Use the import button in the object library to add FBX, OBJ or glTF files (furniture, people or cars from any site you have rights to, or Revit families exported to FBX).
  - They're listed under All objects › Imported, kept for every project, and placed, moved, rotated and scaled like any other object.
  - Units and axes are read from the file. glTF roughness and metalness maps are used.

### Movie mode (film icon in the sidebar)

- **Clips.** Each clip is a camera path: move to a shot and press **+ Keyframe**, move again and add more. The camera follows a smooth curve through the keyframes and turns the short way round.
- **Previewing.** Play the clip or scrub through it. Click a keyframe to jump to it; right-click it to update or delete it.
- **Several clips.** Add clips with the filmstrip along the bottom; clips are saved in the project.
- **Rendering.** The blue film button renders at 720p, 1080p or 4K and 24, 30 or 60 fps, at the highest quality, with a progress bar and a Cancel button. Each clip becomes a folder of PNG frames. If `ffmpeg.exe` sits next to `RenderMill.exe` or on your PATH, an MP4 (H.264) is made too.
- **Effects.** Photo-mode effects apply to movies, and clouds, water and plants animate over the clip.
- **Command line:** `lumencraft --movie project.lcproj out-folder [--size 1920x1080] [--fps 30]`.

## 0.6

### Rendering

- **Physically based sky.** A Rayleigh and Mie atmosphere is ray-marched into a sky lookup table every frame. The background, reflections, ambient light and fog all sample it, so sunsets, haze and overcast skies stay consistent across the image.
- **Clouds.** Layered cumulus clouds shade themselves by marching toward the sun, with silver linings when the sun is behind them.
- **Mountain backdrop.** A ring of distant mountains surrounds the site, with forest, meadow, rock and snow chosen by height and slope. Aerial perspective fades them into the sky. Toggle them under Landscape › Mountains.
- **3D grass.** Grass grows around the model, roughly 700k blades. It stays clear of floors and paving and grows in patchy clumps. Toggle it and set the height in the Landscape tab. Grass is hidden at viewport quality 1.
- **Photo-textured landscapes.** The ground types are Lawn, Meadow, Dry grass, Forest floor, Gravel and Sand, each with tiling break-up.
- **Screen-space reflections.** Glass, water and glossy floors, stone, metal and car paint reflect the actual scene, falling back to the sky where the reflected ray finds nothing. Reflections need viewport quality 2 or higher and the *Screen-space reflections* effect.
- **Ground bounce light.** Light bounced off the sunlit ground fills in eaves, soffits and other downward-facing surfaces.
- **Trees and plants.** Trees, conifers, shrubs and planters are built from hundreds of alpha-cut leaf cards. They cast dappled shadows and keep their density at a distance.
- **Furniture and cars.** Both now use rounded geometry, and cars have rims and lights.

### Photo mode

- **New effects:** *Depth of field* (auto-focus or a set distance), *Chromatic aberration*, *Film grain*, *Auto exposure* and *Screen-space reflections*.
- **Supersampling.** Final renders are drawn 1.5–2× larger and filtered down, which gives cleaner edges, foliage and fine detail.

### Material library (Material editor › Library)

- **Materials.** About 270 free CC0 PBR materials ship with the app, each with colour, normal and roughness maps and a real-world size. They come from ambientCG and are grouped into 23 categories: brick, tiles, wood, wood floor, siding, concrete, paving, stone, marble, metal, roofing, plaster, fabric, ground, asphalt and others. The *Basic* category holds glass, water and light.
- **My folders.** Your own texture folders are indexed in place; nothing is copied. Add or remove folders in Settings › Material library folders. Render Mill also picks up `OneDrive\Documents\Work\WHD\Rendering\1. Lumion Textures` automatically on first start.
  - Each sub-folder becomes one material, named after the folder.
  - Files named `-COLOR-`, `-NORMAL-`, `-ROUGHNESS-` or `-GLOSS-` are recognised, as in manufacturer and Arroway-style packs.
  - Previews, `COLOR_WITH_SHADOWS`, displacement and reflectivity maps are skipped.
  - Loose images next to the folders become colour-only materials.
  - Categories are guessed from the name (Siding, Brick, Stone, Metal and so on).
- **Search and thumbnails.** You can search the library. Thumbnails load in the background, only for materials that are on screen.

