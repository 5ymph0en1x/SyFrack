"""Compose the two visually selected Julia artworks from the complex salon."""
import importlib.util
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location(
    "salon", Path(__file__).with_name("compose-complex-studies.py")
)
salon = importlib.util.module_from_spec(spec)
spec.loader.exec_module(salon)

SELECTION = (
    ("02-julia-soie", "soie-de-feu", "Soie de feu"),
    ("20-julia-glace", "nacre-boreale", "Nacre boréale"),
)

if __name__ == "__main__":
    folder = ROOT / "scenes/studies/soies-fractales-20261013"
    folder.mkdir(parents=True, exist_ok=True)
    records = []
    for source, name, title in SELECTION:
        original = next(s for s in salon.STUDIES if s["name"] == source)
        study = original | {"name": name}
        (folder / (name + ".ron")).write_text(salon.compose(study), encoding="utf-8")
        records.append(study | {"title": title, "source_study": source})
    (folder / "selection.json").write_text(
        json.dumps(records, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    print(f"{len(records)} selected native scenes in {folder}")
