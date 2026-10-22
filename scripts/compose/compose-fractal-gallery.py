"""Build an offline gallery from SyFrack's native renders without editing pixels."""
import html
import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location(
    "salon", Path(__file__).with_name("compose-complex-studies.py")
)
salon = importlib.util.module_from_spec(spec)
spec.loader.exec_module(salon)
OUTPUT = ROOT / "renders/studies/soies-fractales-20261013"

NOTES = {
    1: "Plumes d'azur et fond perlé ; une piste vers la version nacrée.",
    2: "Retenue : grandes plumes cuivrées, récursion dense et couleurs liées.",
    3: "Une belle branche ramifiée ; le cadre garde de larges plages calmes.",
    4: "Les bandes chaudes occupent trop de place pour ce tissu recherché.",
    5: "Un motif d'empreinte intéressant, moins souple que les plumes de Julia.",
    6: "Le cuivre souligne les bandes ; les grandes zones restent peu détaillées.",
    7: "Pétales récursifs ; le centre concentre trop de bandes serrées.",
    8: "L'inversion étire des chaînes d'or autour d'une étoile.",
    9: "Velours coloré et frontières ornementales ; centre trop brouillé.",
    10: "Nacre en éventail ; les transitions de ce premier essai sont inégales.",
    11: "La mémoire de Phoenix produit une structure intéressante, encore raide.",
    12: "Une silhouette de dragon, moins cohérente dans ses bandes.",
    13: "Nova dessine des fleurs et des volutes, avec de grandes zones lisses.",
    14: "Une spirale de Julia inversée ; les premiers mélanges sont trop cassés.",
    15: "Des chaînes de corolles cuivrées autour d'un centre très régulier.",
    16: "Une nacre délicate, mais trop de stries se confondent au centre.",
    17: "Un sillage fluide après correction des bandes ; le vide reste dominant.",
    18: "Nova décentrée : beaux accents, grandes plages peu articulées.",
    19: "Newton décentrée : la couture devient trop périphérique.",
    20: "Retenue : le bleu, la perle et l'or révèlent un tissu sur tout le cadre.",
    21: "Le détail de Nova grossi garde de beaux pétales, mais un fond trop diffus.",
    22: "Le détail de Newton révèle ses chaînes ; le disque central reste raide.",
}


def relative(path):
    import os
    return Path(os.path.relpath(path, OUTPUT)).as_posix()


def card(image, title, family, note, selected=False):
    stem = image.with_suffix("")
    links = " · ".join(
        f'<a href="{html.escape(relative(stem.with_suffix(ext)))}">{label}</a>'
        for ext, label in ((".png", "PNG"), (".exr", "EXR"), (".ron", "Scène RON"))
    )
    url = html.escape(relative(image))
    tags = family + (" Sélection" if selected else "")
    return f'''<article data-family="{tags}">
<a class="picture" href="{url}" target="_blank" rel="noopener"><img src="{url}" alt="{html.escape(title)}" loading="lazy"></a>
<div class="caption"><span class="family">{family}{' · Retenue' if selected else ''}</span>
<h3>{html.escape(title)}</h3><p>{html.escape(note)}</p><div class="links">{links}</div></div></article>'''


def main():
    OUTPUT.mkdir(parents=True, exist_ok=True)
    heroes = []
    for name, title, note in (
        ("soie-de-feu", "Soie de feu", "Cuivre, corail et turquoise : des plumes sur une trame de rosaces."),
        ("nacre-boreale", "Nacre boréale", "Bleu profond, perle et or : un tissu froid, dense et lumineux."),
    ):
        image = OUTPUT / (name + ".png")
        if not image.exists():
            raise SystemExit(f"Render the selected artwork first: {image}")
        heroes.append(card(image, title, "Julia", note, True))

    studies = []
    folders = [
        ROOT / "renders/studies/complex-crops-20261013",
        ROOT / "renders/studies/complex-refinements-20261013",
        ROOT / "renders/studies/complex-salon-20261013",
    ]
    for study in salon.STUDIES:
        image = next((p / (study["name"] + ".png") for p in folders
                      if (p / (study["name"] + ".png")).exists()), None)
        if image is None:
            continue
        number = int(study["name"][:2])
        title = study["name"].replace("-", " ")
        studies.append(card(image, title, study["family"], NOTES[number]))

    filters = "".join(f'<button data-filter="{family}">{label}</button>' for family, label in (
        ("all", "Toutes"), ("Julia", "Julia"), ("Mandelbrot", "Mandelbrot"),
        ("BurningShip", "Burning Ship"), ("Newton", "Newton"), ("Nova", "Nova"),
        ("Phoenix", "Phoenix"),
    ))
    document = '''<!doctype html><html lang="fr"><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Soies fractales · SyFrack</title><style>
:root{color-scheme:dark;font-family:system-ui,sans-serif;background:#090e14;color:#dce8e9}
*{box-sizing:border-box}body{margin:0}main{max-width:1500px;margin:auto;padding:50px 28px 60px}
.eyebrow,.family{font-size:.72rem;letter-spacing:.15em;text-transform:uppercase;color:#b5a77d}
h1{font-family:Georgia,serif;font-weight:400;font-size:clamp(2.5rem,6vw,5rem);margin:12px 0}
.intro{font-size:1.05rem;max-width:750px;color:#a5b9c1;line-height:1.7}
h2{font-family:Georgia,serif;font-size:2rem;font-weight:400;margin:52px 0 20px}
.hero,.grid{display:grid;gap:22px}.hero{grid-template-columns:repeat(2,minmax(0,1fr))}
.grid{grid-template-columns:repeat(3,minmax(0,1fr))}article{background:#111a23;border:1px solid #22333e;border-radius:10px;overflow:hidden}
.picture{display:block}.picture img{width:100%;height:auto;display:block;aspect-ratio:1.618;object-fit:contain;background:#070b11}
.caption{padding:20px}h3{font-family:Georgia,serif;font-weight:400;font-size:1.45rem;margin:8px 0}
.caption p{color:#9eb4bf;line-height:1.6;font-size:.9rem;min-height:2.8em}.links{font-size:.8rem}
a{color:#81cbcf;text-decoration:none}a:hover{text-decoration:underline}
nav{display:flex;flex-wrap:wrap;gap:8px;margin:0 0 24px}button{background:#172630;border:1px solid #344853;color:#c8d9dd;border-radius:30px;padding:9px 17px;cursor:pointer}
button.active{background:#b8d6cf;color:#10232b;border-color:#b8d6cf}article[hidden]{display:none}
footer{border-top:1px solid #233440;color:#8ca5b2;font-size:.85rem;line-height:1.7;margin-top:45px;padding-top:24px}
@media(max-width:950px){.grid{grid-template-columns:repeat(2,minmax(0,1fr))}}
@media(max-width:650px){main{padding:30px 16px}.hero,.grid{grid-template-columns:1fr}}
</style><main><div class="eyebrow">SyFrack · 13 octobre 2026</div>
<h1>Soies fractales</h1><p class="intro">Des équations classiques aux tissus imaginaires.
Six familles explorées, vingt-deux compositions comparées, deux images retenues pour leurs courbes,
leur densité et leurs couleurs. L'esthétique décide de la sélection.</p>
<h2>La sélection</h2><div class="hero">''' + "\n".join(heroes) + '''</div>
<h2>Le carnet d'exploration</h2><nav aria-label="Filtrer les familles">''' + filters + '''</nav>
<div class="grid">''' + "\n".join(studies) + '''</div>
<footer>Rendus mathématiques natifs de SyFrack. Sélection : 2330 × 1440 pixels, 16 échantillons par pixel,
PNG 16 bits et EXR linéaire. Cliquer une image ouvre sa pleine définition ; chaque scène RON est rééditable.
Les notes expriment le choix artistique de cette série.<br>
<a href="../../../docs/fractales-complexes-et-recherche-esthetique.md">Équations, paramètres et méthode</a>
</footer></main><script>
const buttons=[...document.querySelectorAll('button[data-filter]')];
function filter(value){buttons.forEach(b=>b.classList.toggle('active',b.dataset.filter===value));
document.querySelectorAll('.grid article').forEach(a=>a.hidden=value!=='all'&&!a.dataset.family.split(' ').includes(value));}
buttons.forEach(b=>b.addEventListener('click',()=>filter(b.dataset.filter)));filter('all');
</script></html>'''
    (OUTPUT / "galerie.html").write_text(document, encoding="utf-8")
    print(f"2 selected artworks + {len(studies)} studies: {OUTPUT / 'galerie.html'}")


if __name__ == "__main__":
    main()
