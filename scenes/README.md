# Example scenes

Open a `.ron` file in Studio or render it with the CLI. The file contains
geometry, camera, lights, materials and image-development settings.

| Starting point | Purpose |
| --- | --- |
| `example.ron` | Simple sphere scene |
| `mandelbulb.ron` | Recursive bulb form |
| `mandelbox.ron` | Folded box fractal |
| `menger.ron` | Recursive sponge |
| `exhibition.ron` | Composed form and lighting fixture |
| `environments/` | Complete architectural, ocean and nebula scenes |
| `materials/` | Material and independent texture examples |
| `showcase/` | Selected native 2D and 3D compositions |

Historical French filenames and artistic names are preserved as stable project
identifiers. `.syf` files are Forme construction sources; legacy French syntax
remains accepted, while newly exported source uses English keywords.

Some complete compositions are deliberately demanding. Start with a small
output and low sample count before committing to a large render. External HDRI
paths are relative to the repository and use the assets in `assets/hdri/`.
