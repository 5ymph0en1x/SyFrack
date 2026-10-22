# Fractales complexes et recherche esthétique

Le nouveau mode de texture évalue des nombres complexes dans le plan XY.
Il complète les graphes de géométrie 3D et évite de confondre puissance
sphérique et puissance complexe. Les textures gardent l'éclairage, les
matériaux, le relief filtré et les exports natifs de SyFrack.

## Familles disponibles

Avec `p` entier, les modèles utilisent les récurrences suivantes :

| Famille | Récurrence de base |
| --- | --- |
| Julia | `z ← z^p + c`, avec le pixel comme point initial |
| Mandelbrot | `z ← z^p + c`, avec `z = 0` au départ et le pixel comme constante |
| Burning Ship | `z ← (abs(Re(z)) + i abs(Im(z)))^p + c`, initialisé à zéro |
| Newton | `z ← z − λ (z^p − 1) / (p z^(p−1))` |
| Nova, variante Julia | Le pas de Newton précédent, puis ajout d'une constante complexe `c` |
| Phoenix | `z[n+1] = z[n]^p + c + m z[n−1]`, avec `z[−1] = 0` |

Les modèles de base emploient une torsion nulle. La torsion multiplie les
pas polynomiaux par `exp(i θ)` ; pour Newton et Nova elle donne une
relaxation complexe `λ = r exp(i θ)`. Une inversion du cadre
remplace le point `w` par `k w / max(abs(w)^2, 10^-8)`, avant son décentrage.
Ces déformations et leurs compositions sont des variantes artistiques.
Elles ne sont pas présentées comme de nouvelles découvertes mathématiques.

Les divisions singulières sont régularisées et les calculs sont en float32
sur le GPU. Les études proposées restent à des zooms modestes ; il ne s'agit
pas d'un moteur de zoom profond à précision arbitraire.

## Coloration

Les quatre canaux observés sont :

1. Le temps d'échappement lissé, ou le temps de convergence de Newton, divisé
   par le budget d'itérations. Pour les polynômes, le lissage utilise
   `n + 1 − log(log(abs(z))) / log(p)`.
2. Le minimum de la distance de l'orbite à un anneau de rayon réglable,
   comprimé dans [0, 1].
3. La moyenne des bandes `0,5 + 0,5 sin(s arg(z[n]))`, interpolée sur la
   dernière génération pour atténuer les paliers d'échappement.
4. L'angle du dernier point complexe, ramené dans [0, 1].

Le mode **Mélange des orbites** combine ces canaux avec des poids signés,
puis les palettes cycliques créent les liserés. Le canal Distance d'une
texture complexe décrit le rayon final comprimé de l'orbite ; il ne constitue
pas une distance géométrique à une surface.

Dans Studio : panneau **Texture**, activer **Fractale complexe**, choisir
l'équation, puis régler constante, cadre, relaxation/torsion et bandes.
L'option est facultative : les anciens projets restent des textures par graphe.

## Laboratoire reproductible

```powershell
python scripts/compose/compose-complex-studies.py
powershell -ExecutionPolicy Bypass -File scripts/build-laptop.ps1 -BuildComplexLab
target/laptop/laptop/examples/complex_lab.exe scenes/studies/complex-salon-20261013 renders/studies/complex-salon-20261013 800 500 8
```

Le laboratoire partage une compilation GPU entre les études. Chaque image
conserve son PNG 16 bits, son EXR brut et son projet RON. Il refuse d'écraser
une image existante. Les formules, palettes et paramètres sont conservés dans
`scripts/compose/compose-complex-studies.py` et `studies.json`.

Une correction du mélange des bandes garantit la continuité quand un pixel
s'échappe une itération avant son voisin. La moyenne inclut le point échappé,
avec un poids lié au logarithme du rayon et au seuil d'échappement, plutôt
qu'à la seule partie fractionnaire du compte d'itérations. Un test traverse
explicitement cette frontière sur `z ← z^2`.

La sélection est visuelle : articulation des courbes, cohérence des couleurs,
richesse du détail et absence de grandes plages brouillées. Une formule plus
compliquée n'est pas privilégiée si son image est moins intéressante.

## Sélection du 13 octobre 2026

Vingt-deux compositions ont été examinées, puis deux Julia retenues :

- **Soie de feu** : `z ← z² − 0,8 + 0,156 i`, cadrage tourné de 23°,
  bandes angulaires de fréquence 8, palette iridescente répétée 13 fois.
  Les plumes cuivrées et turquoise traversent un fond de petites rosaces.
- **Nacre boréale** : `z ← z² − 0,74543 + 0,11301 i`, bandes de fréquence 17,
  palette bleue, perle et or répétée 13 fois. Le mélange des anneaux est
  plus prononcé pour donner un tissu froid et scintillant.

Les essais de Newton et Nova ont produit des chaînes de pétales et des
corolles. Leurs grands aplats et leurs centres parfois brouillés les ont
écartés de cette première sélection. Les variantes par inversion et torsion
restent disponibles dans le salon pour poursuivre la recherche.

Les scènes retenues sont dans `scenes/studies/soies-fractales-20261013/`.
Le rendu natif utilise 2330 × 1440 pixels, au rapport 233:144 issu de la suite
de Fibonacci, et 16 échantillons par pixel. Chaque œuvre conserve un PNG
16 bits, un EXR linéaire et sa scène RON. Le relief est un matériau procédural
sur un plan, calculé par SyFrack.

Le rapport `233/144 ≈ 1,618056` approche le nombre d'or φ. La fréquence 13
de la palette, ainsi que les 8 bandes de **Soie de feu**, proviennent aussi
de Fibonacci. Les angles d'orbite utilisent `atan2`, une normalisation par
`2π` et des bandes sinusoïdales ; la trigonométrie agit donc directement
sur le dessin des couleurs.

```powershell
python scripts/compose/compose-soies-fractales.py
powershell -ExecutionPolicy Bypass -File scripts/render/render-soies-fractales.ps1
python scripts/compose/compose-fractal-gallery.py
```

La galerie locale `renders/studies/soies-fractales-20261013/galerie.html`
présente la sélection et les vingt-deux études. Pour refaire un rendu,
passer un nouveau dossier avec `-Output` ; le laboratoire refuse d'écraser
les images existantes. Dans Studio, ouvrir
`scenes/studies/soies-fractales-20261013/soie-de-feu.ron` pour retrouver
**Soie de feu**. Les raccourcis locaux de l'atelier d'origine ne sont pas distribués.

## Repères

L'[analyse de Cleve Moler sur les fractales de Newton](https://blogs.mathworks.com/cleve/2016/01/18/fractal-global-behavior-of-newtons-method/)
présente directement l'itérateur, la convergence et la coloration par angles
et nombre d'itérations. La [galerie d'Arnaud Chéritat](https://www.math.univ-toulouse.fr/~cheritat/fractal/fractal.html)
montre la richesse visuelle des ensembles de Julia et des régions de Mandelbrot.
Les études de SyFrack sont des compositions originales, avec leurs propres
paramètres et palettes.
