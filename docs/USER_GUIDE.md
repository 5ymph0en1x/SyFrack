# SyFrack user guide

## Your first session

Launch Studio from the repository root. Open a `.ron` scene with **File → Open**
or pass its path as the first argument to `syfrack-studio`.
For example, `scenes/mandelbulb.ron` is a useful 3D starting point.

Switch between **2D** and **3D** in the command bar to change the projection of
the same scene, file and undo history. Connect geometry, patterns, GPU flames
and natural fields in **Graph**. There is one Scene output and a common export.
Switching interface language does not save or modify a scene.

## Choose your interface

The command bar always offers **Interface → Simple / Advanced**. Advanced is
the first-launch default. Your choice is saved locally in
`%APPDATA%/SyFrack/interface.txt` on Windows, independently of scene files.

**Simple** focuses on the current creative goal. In 2D, work through **Pattern →
Colours → Framing**; in 3D, use **Volume → Material → Lights → Image**. Pattern
families, palettes and volume controls edit the same connected graph as Advanced.
When several sources contribute to an image, choose the source to adjust.
If a 3D scene has no connected colour source, choosing **2D** in Simple adds
an editable pattern and palette while retaining the geometry. Undo restores
the previous composition in one step.

**Advanced** keeps direct navigation to **Workshop**, **Graph**, **Mosaic** and
**Gallery**. In Simple, **Tools** opens Mosaic or the advanced graph, and each
inspector includes **Open the advanced graph**. Specialized sources and unfinished
wiring remain editable there. Render/export, file actions and undo/redo remain
available in either interface.

Switching interfaces preserves the scene, file, graph selection and undo/redo
history. Switching from the graph to Simple returns to the workshop. Choosing
2D or 3D still changes the scene projection and remains undoable.

In **Graph**, use **Library → Path** to find, select and frame an existing stage,
or **Add** to extend it. Selecting a socket filters the available nodes to
compatible operations. Connections work in either direction and can also be
chosen from the inspector. **Compose** adds complete editable pattern, wave
interference or flame branches; undo restores the previous connections.
**Focus path** dims unrelated branches without changing the render. Press **F**
to frame everything, **Shift+F** to frame the selection, **Shift+A** to search
for a node, and **Shift+Delete** to remove unary operations while reconnecting
compatible neighbours. Source resolution and simulation budgets are grouped
under **Source precision**. See the [detailed nodal guide](atelier-nodal.md).

Choose **View → Language → English / Français**. The preference is stored in
`%APPDATA%/SyFrack/language.txt` on Windows. `SYFRACK_LANG=en` or `fr` overrides
the saved choice at startup. The catalog is embedded; an internet connection is
not needed for translation.

## 2D: patterns, colours and framing

1. Choose a family: Julia, Mandelbrot, Burning Ship, Newton, Nova, Phoenix, or
   a fractal flame scene.
2. Adjust the mathematical pattern before framing the final composition.
   Changes to the complex constant, power and inversion can transform the image.
3. Choose a palette, adjust its rhythm and phase, then edit individual stops.
   Smooth transitions and cyclic sampling can help continuous colour flow.
4. Drag to pan and scroll to zoom around the pointer. Rotation turns the frame.

The flat canvas samples the connected colour field independently of 3D lights.
Add a flame source in **Graph** and connect its colour or density to the same
operators as a pattern or simulation. Set its atlas **resolution**, total **points**
and reconstruction filter on the source node. These resource budgets are
independent of final image samples. The graph checks source working memory
before preparing the GPU data. Freeze GPU sources to embed their exact states.
The Scene node also contains domain, optics, animation and variant-atlas tools.

![English 2D colour workshop](images/studio-2d-en.png)

## 3D: volume, material, lights and image

Start with a volume family, then adjust recursive detail, folds and proportions.
Use **Graph** to combine form operations with the same pattern and natural-field sources. The graph provides
primitives, spatial transforms, iteration, traps, CSG and bounded detail.
Names in an existing project are its content and are preserved across languages.

The 3D preview starts in **GPU control** mode. Connecting nodes updates a small
GPU program without compiling another shader. This neutral view retains the
graph's complete geometry, iteration counts, clipping and instance transforms.
It progressively resolves depth and normals, continuing unfinished rays across
GPU passes until they finish. Large programs can reduce preview resolution
to stay within the control memory budget. Incomplete graphs keep the last valid
shape with an explicit caption.

Click **Compile specialized preview** below the node preview or in the workshop
status bar to inspect lighting, materials and volume effects. Control remains
available while it compiles. The clay/final-look switch appears after this
specialization; the control view always uses neutral shading.
**Return to GPU control** restores the fast editing
mode; a structural graph change also returns to it automatically. Final exports
always use the production renderer.

Set base colour, roughness and metallic response in **Material**. Independent
fractal textures can vary colour, roughness and fine relief. Bump and parallax
change surface appearance; they do not add silhouette geometry. Concrete and
puddle patterns are procedural and can be adjusted without external textures.

Add sun, area, neon tube, projector or HDRI lights. Large emitters generally
produce softer shadows. The **Atmosphere** controls include fog and procedural
sea/sky. Volumetric nebulae and fluid preparation are experimental and may be
expensive. **Image** adjusts exposure, tone mapping, colours and neon softness.

Drag to orbit, scroll to zoom, middle-drag to pan. In free-camera mode,
right-drag looks around; WASD or ZQSD moves, E/C moves up/down.
Right-click a surface for focus, orbit centre or light placement actions.

## Mosaic: turn a photograph into fractals

Open **MOSAIC** in the command bar and choose **Open image / project…**. Import
a PNG, JPEG, BMP or TIFF, or reopen a `.mosaic-project.ron` exported by this tool.
Adjust exposure, contrast and saturation, draw a crop with **Crop with the mouse**,
and rotate in quarter turns. These edits use an independent undo history.

Choose a quality starting point and adjust the analysis resolution, detail level,
region sizes and region budget. **Adjusted image** and **Subdivision** update in
the background. Click **Generate / Update** (Ctrl+Enter) for the complete fractal
reconstruction; cancel leaves the previous result available. Use **Compare** and
the before/after slider, scroll to zoom, drag to pan, or double-click to fit.

Under **Preserve details**, **Error refinement** directs more regions to poorly
reconstructed areas (100 by default; 0 restores source-only subdivision).
**Paint** prioritizes selected features; **Erase** removes local priority.
Each stroke is one undo step. Marks follow crop and rotation and are saved in
the project. The fast subdivision preview is provisional until generation.

Changing settings marks the result as stale. Regenerate before exporting. Under
**Destination and enlargement**, choose the name, directory and integer scale,
then **Export mosaic…** (Ctrl+S). The export preserves existing files and saves
a 16-bit PNG, subdivision map, adjusted source, working original, fractal recipe
and portable project. Keep the files together to reopen the project. The working
original is bounded to a 2,048-pixel longest edge; the input file is preserved.
More details and CLI examples are in the [mosaic guide](mosaique-fractale.md).

## Final rendering

Open **Render / Export**. Choose resolution, orientation, quality and destination.
The window groups image format and quality beside the destination and the exact
PNG, EXR and scene filenames. **Custom settings** contains the detailed sampling,
light-bounce and scheduling controls. The launch/pause action stays visible below
the scrolling settings. During rendering, progress, elapsed/remaining time and
the rendered thumbnail take priority; the settings used remain available.
When the job finishes, the thumbnail is refreshed from the complete final image,
including the last tiles and optional denoising, before the status becomes Done.
Presets are starting points; inspect their sample and bounce counts before a
long job. Studio refuses to overwrite an existing finished render silently.

During a final render, the interactive preview keeps its last image and suspends
tracing, tone mapping, resizing, pipeline adoption and depth/pick readbacks.
Scene edits and new library thumbnail jobs wait until the export stops or pauses;
the preview then resumes automatically with the latest edits. The render window
continues to update progress and periodically displays the export's own image.

| Control | Effect |
| --- | --- |
| Samples per pixel | More samples reduce stochastic noise in 3D and improve 2D antialiasing |
| Light bounces | 0 gives direct light; more bounces add indirect transport and reflections |
| Tracing tolerance | Smaller pixel tolerances preserve small geometric openings |
| Tracing steps | Larger limits help deep interiors and difficult recursive surfaces |
| Denoise PNG | Reduces final PNG noise; the raw EXR is preserved |
| Guiding | Experimental learned proposal for indirect light in long renders |
| Tile and batch limits | Control scheduling and update responsiveness, not artistic sharpness |

Final exports contain `name.png` (16-bit), `name.exr` (linear HDR) and `name.ron`
(scene snapshot). Pause preserves a `name.state/` folder. Keep the entire state
folder together and resume with the matching engine build. Increasing quality
in the open editor does not alter the scene embedded in a paused job.

The renderer works in RGB, with approximations for some surface and volume
effects. More samples cannot correct an unsuitable composition, clipped look,
insufficient geometry steps or an approximation's systematic bias.

## Command-line examples

Commands below assume the binaries are on PATH; otherwise use
`target/release/syfrack.exe` on Windows or `cargo run -p syfrack-cli --`.

```text
syfrack --version
syfrack probe
syfrack example-scene --kind mandelbulb --out scenes/my-bulb.ron
syfrack render scenes/my-bulb.ron --out renders/my-bulb --width 1280 --height 720 --samples 64 --bounces 4 --steps 512 --pixel-tolerance 0.5 --denoise --checkpoint-every 60 --preview-every 30
syfrack info renders/my-bulb.state
syfrack resume renders/my-bulb.state --out renders/my-bulb
syfrack --help
```

For a safe unattended pause, supply `--pause-file renders/pause-request` when
starting a CLI job, then create that file. The renderer saves its outputs and
checkpoint. Remove the request file before intentionally resuming with the same
option. `--dispatch-ms` changes scheduling latency without changing accumulated
samples; the supported range is 50–800 ms.

`--backend auto` (the default) uses the GPU wavefront integrator for new 3D
exports, with bounded pooled tiles, and never starts a software CPU worker.
`--backend wavefront` selects that engine explicitly. Its ray queues,
intersection, shading and compaction passes remain on the GPU between bounces.
`--backend tiled` uses the previous monolithic shader with bounded GPU tiles,
and `--backend gpu` uses its resident full-image buffers.
`--backend hybrid` requires both a hardware GPU
and an available Vulkan software CPU adapter; its benefit depends on the scene and
drivers. Guiding and
flame scenes keep that renderer in `auto`. Tiled pauses retain completed tiles;
tiles still being computed are recalculated on resume. The complete linear HDR
image is developed once, preserving global exposure and effects across tiles.
Native CPU/GPU arithmetic can differ at the bit level. For repeatability on one
setup, use `--backend tiled` and retain the same build, device and driver for
resume. New tiled checkpoints refuse changed arithmetic implementations.
To deliberately change processors, use `resume ... --backend hybrid`, `tiled`
or `cpu`. This accepts native floating-point differences, keeps completed tiles
and records producer history. CPU-only also works without a hardware GPU;
it is useful for diagnostics and recovery.
`syfrack benchmark-hybrid scene.ron --out report.ron --repeats 3` compares
isolated CPU/GPU and concurrent hybrid throughput on the same scene and sample
budget. It reports median speedup, each device's contribution and measured HDR
errors. Console messages distinguish warm-up, isolated baselines and the hybrid
pass. The report includes host-clock CPU/GPU dispatch overlap; a hybrid claim
fails if either processor contributes no tile or those intervals do not overlap.
The default assembled-image tolerance is 1% of reference RMS plus 1e-4;
`--relative-rmse` and `--absolute-rmse` make the criterion explicit. See
[performance and quality measurements](DETERMINISM.md) for the report's limits.
`syfrack parity --points 100000 --out parity.ron` measures strict graph and
arithmetic parity, saves its report and fails if the gate does not pass; see
[Determinism](DETERMINISM.md) before interpreting that diagnostic as a full-render
or cross-device guarantee.

On Windows, launch with an additional CPU driver using PowerShell 7:
Use a normal, non-administrator PowerShell window. Vulkan ignores driver
environment overrides in elevated processes; the launcher detects and refuses
that configuration before opening Studio.

```powershell
.\scripts\with-vulkan-cpu.ps1
```

With no arguments, the helper opens Studio: it uses the packaged executable or
the most recently built Studio in the local `target` directories. From inside
`scripts`, use `.\with-vulkan-cpu.ps1`. If Studio has not been built, the helper
prints the build command. To run the CLI instead, supply its executable:

```powershell
.\scripts\with-vulkan-cpu.ps1 -Executable .\target\release\syfrack.exe -RenderArgs @('render', 'scenes/mandelbulb.ron', '--out', 'renders/hybrid', '--backend', 'hybrid')
```

The helper finds an installed SwiftShader runtime in Chrome or Edge, or accepts
`-Icd path/to/cpu-driver.json` for another software Vulkan driver. It adds a
temporary manifest to the child process. The same helper can launch
`syfrack-studio.exe` with no render arguments. Studio's default exports remain
GPU-only; the software CPU backend is retained for explicit CLI diagnostics.
WARP is excluded from the CPU backend.
To check CPU discovery without compiling a scene:

```powershell
.\scripts\with-vulkan-cpu.ps1 -Executable .\target\debug\syfrack.exe -RenderArgs @('probe', '--cpu')
```

The render panel shows whether the CPU is preparing, contributing tiles or
unavailable. Its percentage and estimated remaining time account for completed
pixels across all tiles, so a partly rendered image no longer stays at 0%.

The CLI's command names and help are English. `--lang en` / `--lang fr` selects
localized scene diagnostics. Use `--help` on each subcommand for its exact options.

## Shortcuts

| Shortcut | Action |
| --- | --- |
| Ctrl+O | Open project |
| Ctrl+S / Ctrl+Shift+S | Save / Save As |
| Ctrl+Z / Ctrl+Y | Undo / Redo |
| Ctrl+Enter | Render / Export |
| Tab | Image only |
| H | Light handles |
| F1 | Studio help |

Double-click a numeric value for exact entry. Numeric fields accept both a
decimal point and a comma; displayed values follow the selected language.
