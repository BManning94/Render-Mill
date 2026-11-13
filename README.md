# LumenCraft 0.11.0

A real-time architectural renderer written in Rust (wgpu + egui), modeled on Lumion's workflow and interface.

## Interface (v0.5: Lumion 2025 layout)

The layout follows Lumion Pro 2025: a thin custom title bar, a narrow icon sidebar on the left, a full-screen viewport, and flat charcoal panels.

- **Title bar.** Project name, a frame-rate gauge (green, amber or red, with GPU time on hover), quality stars (F1-F4), and minimise / maximise / close. Drag the bar to move the window, double-click it to maximise, and drag the window edges to resize.
- **Left sidebar.** File (new, open, save, import), Build mode, Photo mode, and Imported model / LiveSync. At the bottom are the Activity log, Settings (quality, auto-resolution, walk speed, LiveSync server) and Help (shortcuts).
- **"Main" view pill and search button** (top-left of the viewport). The pill lists saved views and can frame the model. The round search button opens the object library with the search box focused.
- **LiveSync card** (top right). A green dot means a live file or BimCraft link. The power button pauses or resumes LiveSync, or imports a model if none is linked. Under the card is the imported-model card (thumbnail, import a different model, reload, import settings). In the Delete tool this becomes the **Selected objects** card with a red bin.
- **Build dock** (bottom left). It has four tabs: Home, Materials (brush), Landscape and Weather.
  - **Home tab:**
    - The imported-model button.
    - A 2×4 category grid: Nature, People, Transport, Interior, Exterior, Lights, Favourites, All.
    - Select all.
    - The tool block: Place, Select, Delete. Each has sub-modes: single, paint and mass place; move, height, rotate and scale; single and area delete.
    - Random rotation and size.
    - Undo and deselect.
  - **Object library.** Clicking a category opens it on the left, with search, category and sub-category icons, a breadcrumb, and a thumbnail grid. Hovering an item shows a preview card with its tags and a favourite star. Click an item, then click in the view to place it.
  - **Placement options and object properties.** These appear beside the dock: Position X/Y/Z, Heading, Size, and the light settings.
  - **Selection display.** Selected objects get a yellow box and an axis gizmo. Ctrl+click adds to the selection.
- **Material editor** (brush tab).
  - **Picking a surface.** Hover a surface to highlight it in yellow with its name, then click it. The surface being edited is tinted blue.
  - **Panel tabs.** The panel has **Library** and **Settings** tabs.
  - **Value bars.** The bars, as in Lumion, are Color, Texture, Map scale, Relief, Roughness, Reflectivity and Metalness.
  - **Map swatches.** Click a swatch to load a map; hover it to see a large preview with its size and file name.
  - **Show more / Show less** toggles the extra bars.
  - **Sections.** Advanced (type, opacity, glow, cut-out), Placement (X/Y offset, heading, *Enable landscape tiling*), Weathering, and Imported material (reset or Invisible).
  - **Header tools.** Show surfaces, copy and paste material.
  - **Saving.** The blue ✓ in the bottom-right keeps the changes; ✕ discards everything since the editor opened.
- **Photo mode** has its own layout:
  - **Left panel.** The style name, a *Custom style* menu, and *+ Add effect*.
  - **Effect list.** It has a master switch. Each effect has an eye toggle and expands to show its settings when clicked: Color correction, Sky light, Fog, Shadow, Reflection, Bloom, Vignette, Sharpen.
  - **Preview.** A framed preview in the chosen aspect ratio, with a camera bar underneath (focal length in mm, heading, pitch, aspect ratio, camera lock, render settings).
  - **Filmstrip.** 10 photo-set slots along the bottom.
  - **Render button.** A large blue button that offers HD up to 8K, *Render this photo* and *Render photo set*.
- **Shortcut hints** run down the right edge (WASD, QE, O, RMB, MMB, SCROLL, plus M/H/R/L/DEL when objects are selected). While you navigate they expand into labels such as "Shortcut: Speed up".

| Keys | Action |
|---|---|
| M / H / R / L | Move / height / rotate / scale the selection |
| Ctrl+click, Ctrl+A, Esc | Add to selection, select all, deselect |
| Delete | Delete the selection or marked objects |
| Ctrl+Z / Ctrl+Y | Undo / redo |
| Ctrl+S | Save project |
| Ctrl+1..9 / Shift+1..9 | Store / recall a camera |
| F1-F4 | Viewport quality |

## Installing

Extract `lumencraft-0.7.0.zip` and every `lumencraft-library-…-0.7.0.zip` and `lumencraft-models-…-0.7.0.zip` part into the same place ("Extract here"). Together they make one `LumenCraft` folder holding `lumencraft.exe`, `library\` (materials) and `models\` (3D objects). Then run `LumenCraft\lumencraft.exe`.

**Updating:** extract the newest `lumencraft-<version>.zip` into the same place, replacing `lumencraft.exe`.

**Updating from 0.7:** extract `lumencraft-0.8.0.zip` and both `lumencraft-denoiser-…-0.8.0.zip` parts into the same place as before, replacing `lumencraft.exe`. The library and models from 0.7 are unchanged. The denoiser parts add a `LumenCraft\denoiser\` folder (Intel Open Image Denoise, Apache 2.0); its large DLL is split in two and joined automatically the first time you render.

## What's new in 0.13 — start screen, landscape templates, smoother camera

### Start screen and landscape templates
LumenCraft now opens on a start screen like Lumion's: **Create new**,
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

## What's new in 0.12 — GPU limits, texture budget, smoother live preview

### Photo and movie sizes checked against your graphics card
Before a render starts, LumenCraft checks the chosen size against the card's
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
to lumencraft.exe) to record refresh times on your card into
`preview-benchmark.txt`; `lumencraft.exe --bench-preview [scene]` does the same
from a terminal.

### Compatibility
No project-file changes. New optional `texture_budget_mb` in settings.json.
Public API additions: `pathtrace::{check_resolution, max_resolution,
ResolutionLimit, TexturePlan}`, `PathTracer::{try_begin, request_preview,
poll_preview}`, `Renderer::{texture_stats, set_texture_budget,
render_post_only}`. Changed: `pathtrace::plan_classes` takes a data-map flag
list and returns a `TexturePlan`; `CLASS_SIZES` now has six classes
(128 px – 4096 px); `PtMemory` has `dropped`, `smallest_reduced` and `budget`.

## What's new in 0.11 — rendering quality, performance and stability

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

## What's new in 0.10

- **Textures on imported buildings now show at their real size.** Materials from the library (and your own texture folders) are placed in metres on every face — "real-world mapping" — instead of relying on the model's UV coordinates. Revit exports often have UVs that are missing, stretched 0–1 across a whole wall, or in feet/inches, which made textures look smeared, giant, tiny or like a flat colour.
- Material editor › Placement › **Real-world mapping** turns it on or off per material (off = use the model's own UVs, e.g. for a model that was carefully UV-mapped).
- The ray tracer places textures exactly the same way.
- Projects saved with earlier versions switch their library textures to real-world mapping when opened.

## What's new in 0.9

### Faster on your hardware
- **Choose the graphics card** (Settings › Graphics card): pick your eGPU or the laptop GPU by name; LumenCraft remembers it and uses it on the next start. Command-line renders use the same choice (or `LUMENCRAFT_GPU=3090` for one run).
- On a dedicated graphics card the viewport now starts at full quality (4 of 4).
- **Ray tracing is faster (about 2× in testing, more on fast cards):** each GPU dispatch traces many samples per pixel at once (tuned automatically to the card's speed, short enough to never trip the Windows GPU timeout — important over Thunderbolt), and the CPU no longer waits on the GPU between tiles.
- **AI denoiser on the GPU:** Open Image Denoise now keeps one device for the session, so it runs on your NVIDIA card through CUDA without start-up cost each time (the log says which device it uses).

### More photo-realistic
- **Ray-traced preview** in Photo mode (the camera button next to the menu under the preview): while the camera is still, the preview refines itself with path tracing and AI denoising — 1, 2, 4, 8 … samples — so you see the final photo before rendering. Move the camera and it drops back to the real-time view.
- **Filmic tone curve (AgX)**, on by default (Photo › Color correction): bright skies and sunlit walls roll off to white like a real camera instead of shifting hue; colours stay natural. Turn it off for the previous ACES look.
- **Real lens depth of field** in ray-traced photos: with the Depth of field effect on, the path tracer uses a thin-lens camera (round bokeh, correct blur through glass and foliage) focused on the set distance or whatever is in the centre of the frame.
- Softer, more natural 3D grass in ray-traced images.

## What's new in 0.8

### Ray-traced photos with an AI denoiser

Photo mode › Render (camera button) › **Ray tracing**: Off / Fast / High / Ultra / Max.

- A GPU path tracer (compute shader, BVH acceleration) renders the photo with physically based light transport: global illumination (light bouncing between walls, floors and ground), soft sun shadows, true mirror and glossy reflections, refraction and absorption in water, light through thin glass, translucent leaves, alpha cut-outs (leaves, fences), emissive materials and every placed light.
- Same materials, textures, sky, sun, fog and photo effects as the rasterizer, so ray tracing is a switch, not a different look. Exposure, bloom, depth of field, lens effects and colour grading are applied afterwards as usual.
- **AI denoiser:** the remaining grain is removed with Intel Open Image Denoise, a neural network trained on path-traced images, guided by albedo and normal layers so texture detail and edges stay sharp. Runs on the CPU (or CUDA on NVIDIA). If the `denoiser` folder is missing, an edge-aware wavelet filter is used instead.
- Fast = 16 samples/pixel, High = 64, Ultra = 256, Max = 1024. A progress card shows samples and time left; Cancel at any time. Photo sets render one after another.
- Movie mode › Render › Ray tracing renders every frame path-traced (with a quarter of the samples per frame, denoised).
- Command line: `lumencraft --render model.fbx out.png --raytrace high` (`fast|high|ultra|max|<samples>`, `--no-denoise`).

Ray tracing runs on any GPU through wgpu (DirectX 12 / Vulkan); it doesn't need RTX cores but is much faster on a recent card.

## What's new in 0.7

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
- **Rendering.** The blue film button renders at 720p, 1080p or 4K and 24, 30 or 60 fps, at the highest quality, with a progress bar and a Cancel button. Each clip becomes a folder of PNG frames. If `ffmpeg.exe` sits next to `lumencraft.exe` or on your PATH, an MP4 (H.264) is made too.
- **Effects.** Photo-mode effects apply to movies, and clouds, water and plants animate over the clip.
- **Command line:** `lumencraft --movie project.lcproj out-folder [--size 1920x1080] [--fps 30]`.

## What's new in 0.6

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
- **My folders.** Your own texture folders are indexed in place; nothing is copied. Add or remove folders in Settings › Material library folders. LumenCraft also picks up `OneDrive\Documents\Work\WHD\Rendering\1. Lumion Textures` automatically on first start.
  - Each sub-folder becomes one material, named after the folder.
  - Files named `-COLOR-`, `-NORMAL-`, `-ROUGHNESS-` or `-GLOSS-` are recognised, as in manufacturer and Arroway-style packs.
  - Previews, `COLOR_WITH_SHADOWS`, displacement and reflectivity maps are skipped.
  - Loose images next to the folders become colour-only materials.
  - Categories are guessed from the name (Siding, Brick, Stone, Metal and so on).
- **Search and thumbnails.** You can search the library. Thumbnails load in the background, only for materials that are on screen.

## Features

- **Renderer**
  - PBR (GGX) shading and a procedural sky with animated clouds and stars.
  - Cascaded sun shadows: 4 × 2048 maps that stay sharp up close and still reach large sites.
  - Reverse-Z depth with an infinite far plane, so there's no flicker at distance.
  - SSAO, bloom, ACES tone mapping, height fog and 4× MSAA.
- **Lights:** point and spot lights you can place, plus street lamps that actually light the scene (up to 256 lights).
- **Materials:** Standard, Glass, Water (animated), Emissive, Foliage (wind sway) and Landscape. Revit material names are recognized automatically, and there's a preset library.
- **Material edits persist by name across every re-sync**, just as in Lumion LiveSync.
- **Objects:** procedural trees, conifers, shrubs, planters, boulders, people, cars, armchairs, sofas, dining tables, chairs, park benches, street lamps and point/spot lights. You can place one at a time, paint them along a path, or mass-place them, then move, rotate and scale whole selections.
- **Weather presets:** midday, golden hour, overcast, morning, dusk and night.
- **Photo:** saved views and PNG export up to 4K. Projects save as `.lcproj` (JSON).

## Your own textures (Custom materials)

You can add your own textures in three ways:

- **Material editor → Library → Custom → Images… or Folder…** (or *Import textures…* in the brush tab of the dock)
- **Drag image files or whole folders** from Explorer onto the window.
- **Choose… in a material's map slot.** The image is added to the library too.

**Texture sets.** Files are grouped into sets by name. A colour map, normal map and roughness map that share a base name become one set:

- **Poly Haven:** `brick_wall_001_diff_4k`, `_nor_gl_4k`, `_rough_4k`.
- **ambientCG:** `Bricks076C_4K_Color`, `_NormalGL`, `_NormalDX`, `_Roughness`.
- **Generic:** `_albedo`, `_basecolor`, `_normal`, `_gloss`, and so on.

DirectX normal maps and gloss maps are converted automatically, and AO, height and metalness maps are skipped.

**Using a texture:**

1. Click a surface.
2. Click a texture tile to apply it.
3. Set its real-world size with **Map scale** in the material editor, or right-click a tile to set the tile size.

The library is saved with the project.

## Performance

- **Viewport quality, from 1 to 4 stars:** set with F1–F4 or the stars at the top right.

  | Level | MSAA | Shadow maps | Other |
  |---|---|---|---|
  | 1 | Off (FXAA instead) | 1K | No SSAO |
  | 2 | Off (FXAA instead) | 2K | Light SSAO |
  | 3 (default) | 4× | 2K | Full lighting |
  | 4 | 4× | 4K | Maximum |

  Photos always render at level 4.
- **Auto res:** while the camera or an object moves and the frame rate drops, the view renders at lower resolution. It snaps back to full quality as soon as you stop.
- **No wasted frames:** when nothing changes, no frame is rendered at all. When only clouds and water are animating, the view is capped at 30 fps.
- **Renderer optimizations:**
  - Shadow maps are cached and only re-rendered when needed, with far cascades refreshed less often while moving.
  - The ground doesn't render into shadow maps.
  - The sky only draws where it's visible.
  - Opaque surfaces draw front to back, with a depth pre-pass so lighting runs once per visible pixel.
  - Surface noise is cheaper and fades with distance.
  - Shadow filtering uses 12 samples instead of 25.
- **Hover highlights** only test placed objects, never the whole model.
- **`lumencraft --bench <model|--demo> --quality 1-4`** prints frame timings.

## Textures in imported models

Textures are found wherever the model put them:

- **Embedded in the FBX**, whether binary or ASCII, when "Embed media" is ticked.
- **Next to the model**, or in its `.fbm` folder or any sub-folder.
- **In texture folders you add** under Import → Texture folders.
- **In the Autodesk material library**, for Revit's built-in materials.

FBX files often store absolute paths from the computer that exported them, such as `D:\projects\...`. Those are matched by file name, and also by the same name with a different image extension.

**Supported formats:** PNG, JPG, TGA, BMP, TIF, GIF, WebP and DDS.

**Supported maps:**

- **Colour:** diffuse/base colour.
- **Normal:** used as-is, or converted from a greyscale bump map.
- **Opacity/transparency:** becomes a cut-out, for leaves and perforated panels. Cut-outs cast correctly shaped shadows.
- **Placement:** the FBX texture's tiling and offset are applied.

You can also assign or replace any colour or normal map yourself in the material editor. Anything the model references but can't be found is listed in the Import panel.

## Getting a Revit model in (live)

**FBX is Revit's built-in export, so no add-in is needed.**

1. In Revit, choose **File → Export → FBX** and save into a folder, for example `C:\Renders\MyHouse\model.fbx`.
2. In LumenCraft, open the **Import** category (bottom of the left bar) and click **Import model…**. You can also pick **Watch folder…**, which always shows the newest file in that folder.
3. Edit materials, place objects and lights, and set up views.
4. Change the design in Revit and export again over the same file. LumenCraft reloads it automatically and keeps your materials, objects and camera.

How the re-sync works:

- The model is split into spatial chunks. Only chunks whose geometry actually changed are re-uploaded to the GPU, and the activity log reports "N of M chunks changed".
- FBX units and up axis are read from the file itself, so Revit's feet and Z-up are handled automatically.
- glTF/GLB (Y-up meters) and OBJ (defaulting to Z-up feet) also work. You can override any of these under **LiveSync → Import settings**.
- Revit material textures are loaded from the paths saved in the FBX, which is normally the Autodesk material library on the same PC.

Revit still needs an export click. Updating without one would require a Revit add-in, and Revit add-ins must be written in .NET.

## LiveSync TCP protocol (BimCraft / scripts)

The server listens on `127.0.0.1:47811` by default.

**Other computers:** tick "Allow other computers on the network" and they must send the pairing code shown in the panel. Connections from the same PC never need it. A remote client can't make LumenCraft read local files, and frames are capped at 256 MB.

**Protocol:** the spec and a client live in `src/protocol.rs`. The file depends only on serde, so BimCraft can copy it directly. Meshes are upserted by element id, so the host only needs to send the elements that changed.

The `lumencraft-sync` binary is the reference client:

```
lumencraft-sync model.fbx --watch
lumencraft-sync model.fbx --addr 192.168.1.20:47811 --token 123456-789012
```

## Controls

| Input | Action |
|---|---|
| RMB drag | Look around |
| WASD / arrows | Move |
| Q / E | Down / up |
| Shift / Space / both | Fast / slow / extra fast |
| O + RMB drag | Orbit around the point in the screen centre |
| MMB drag | Pan |
| Wheel | Dolly |
| F | Frame the model |
| LMB | Pick a material (Materials mode) or place an object or light (Objects mode) |

## Command-line rendering

```
lumencraft --render model.fbx out.png --size 3840x2160
lumencraft --render project.lcproj out.png --view 0
lumencraft --render --demo out.png --preset "Golden hour" --camera 10,1.7,20,-2,2.5,5
lumencraft --render model.fbx out.png --raytrace ultra
```

## Building

```
cargo build --release
cargo test
```

To cross-compile for Windows from Linux:

```
cargo build --release --target x86_64-pc-windows-gnu
```
