# Nature et cosmos — champs, croissance et optiques

Les champs naturels sont des nœuds du **graphe de création** commun aux formes,
motifs et flammes. Ajoutez un module depuis la bibliothèque du graphe, puis
reliez ses sorties à Scène ou aux autres calculs. Domaine, optiques, chronologie,
états figés et atlas se trouvent sur la sortie Scène. Les anciens documents
`phenomena` sont migrés à l’ouverture ; il n’y a plus d’éditeur autonome.
Voir [le fonctionnement du graphe unifié](graphe-unifie.md).

## Les expériences livrées

Les scènes éditables sont dans `scenes/showcase/nature-cosmos/`. Le programme
`nature_lab` les régénère et produit une galerie PNG, les EXR linéaires, les RON
et les diagnostics numériques dans `renders/nature-cosmos-20261019/`.
Ces études utilisent huit échantillons par pixel, et 96 pour la dispersion. Leurs temps incluent la
préparation des champs et la création initiale des pipelines ; ils ne mesurent
pas le seul débit de rendu du GPU.

| Expérience | Modèle et possibilités |
| --- | --- |
| `interference` | Superposition complexe de paquets d'onde, phase, intensité et volume émissif. |
| `morphogenesis` | Réaction–diffusion de Gray–Scott ; concentrations utilisables pour la couleur, le relief, un masque ou une isosurface. |
| `vortex` | Équation non linéaire de Schrödinger / Gross–Pitaevskii ; germe à enroulement entier, densité et phase. |
| `lightning` | Croissance diélectrique, potentiel harmonique et sélection d'un front connecté. |
| `schottky` | Orbites de générateurs de Möbius appariés, sans annulation immédiate par l'inverse. |
| `quaternion` | Coupe spatiale d'une Julia quaternionique ; la coordonnée de coupe est animable. |
| `resonance` | Modes propres du Laplacien de Dirichlet sur un domaine masqué ; résidu calculé et contrôlé. |
| `iridescence` | Couche mince d'Airy sur substrat diélectrique ou conducteur ; épaisseur modulable par un champ. |
| `kerr` | Géodésiques nulles de Kerr–Schild, capture par l'horizon, intersections et traversée des volumes naturels. `spin = 0` donne Schwarzschild. |
| `dispersion` | Réfraction chromatique avec loi de Cauchy et conservation de la bande choisie à travers les rebonds. |

Ces modèles ne revendiquent aucune découverte scientifique ou exclusivité
artistique. Leur intérêt est leur composition dans le même outil de création.

## Composer une scène

Les sorties des nœuds sont scalaires, vectorielles ou complexes. Les ports sont
colorés ; le validateur refuse les connexions incohérentes et les cycles. Cliquer
une sortie, puis l'entrée de destination, établit la liaison. Un graphe incomplet
reste enregistré dans `editor`, séparément du dernier graphe actif valide : une
connexion en cours n'efface pas la forme affichée. Les déplacements de cartes et
leurs noms ne relancent pas le rendu.

Les opérations couvrent les constantes, les coordonnées, le temps, l'algèbre
réelle/vectorielle/complexe, les ondes, gradient/divergence/rotationnel, les
distances et pièges de la fractale, Möbius, la Julia quaternionique et les canaux
des états simulés. Le produit complexe est une opération explicite.
Dans la coupe quaternionique, faire varier la coordonnée `slice` explore la
quatrième dimension. À `slice = 0`, le premier carré d'un quaternion purement
imaginaire perd son orientation initiale : cette coupe peut donc être sphérique.
Choisir une coupe non nulle pour explorer les structures directionnelles.

Les affectations au rendu sont indépendantes : couleur avec palette éditable,
émission, densité volumique, isosurface, déplacement et épaisseur du film. Une
isosurface peut remplacer, réunir ou intersecter la géométrie initiale. Un même
champ de réaction–diffusion peut ainsi colorer une fractale et en sculpter le relief.

Les entrées scalaires dépendent du modèle : Gross–Pitaevskii accepte potentiel,
source réelle et masque ; Gray–Scott utilise ces champs pour moduler disparition,
apport et domaine. La croissance accepte une pondération du front et un masque ;
les résonances utilisent leur masque géométrique. Le groupe de Schottky conserve
ses générateurs exacts : son résultat peut être masqué dans le graphe de champs.
Les connexions de champs sans effet ne sont pas exposées ni acceptées silencieusement. Les
guides sphériques ajoutent des sources, réservent du vide, protègent une région ou
attirent la croissance. Une simulation peut lire les états antérieurs de la liste,
mais jamais son propre résultat ni un résultat futur. Ce contrat évite les boucles
implicites dont le résultat dépendrait de l'ordonnancement.

**Figer les états calculés** incorpore leurs valeurs dans la scène RON. Les clones
de scène et l'historique partagent ces données immuables en mémoire. Une scène
figée ne relance pas sa simulation à l'ouverture ; retirer le gel permet de la
recalculer. Changer la résolution d'un état figé est refusé jusqu'à son dégel.

Les images clés interpolent explicitement des paramètres numériques : maintien,
linéaire ou transition lissée. L'instant appliqué produit une scène autonome. Le
temps des ondes et le nombre d'étapes d'une simulation sont deux paramètres
distincts ; animer une croissance se fait en animant ses étapes.

L'atlas conserve les scènes parentes et leurs variantes. Choisir les paramètres
à explorer, créer les variantes, calculer leurs vignettes GPU, puis sélectionner
deux vues pour les comparer. Ouvrir une variante la recharge dans Studio. Le
fichier d'atlas conserve la filiation ; son import vérifie les références et les
identifiants. Les vignettes sont des études de 160 × 90 pixels à quatre
échantillons, recalculables ; l'export final utilise les réglages ordinaires.

## Exécution et exactitude

Les stencils de Gray–Scott et Gross–Pitaevskii s'exécutent sur GPU en quatre
kernels RK4 fixes. Les coefficients changent sans recompiler ces kernels. Les coefficients de diffusion sont exprimés dans les unités spatiales du
domaine, et non en cellules. Le pas est subdivisé selon la diffusion et la
fréquence non linéaire. Les soumissions
sont bornées ; une préparation interactive peut céder le GPU à l'export entre
deux soumissions. Une ancienne préparation est annulée lorsqu'une nouvelle
édition la remplace. L'interface conserve son image précédente pendant ce travail.

La préparation des champs, l'initialisation, les croissances de frontière, les
orbites de Schottky et les modes propres utilisent actuellement le CPU. Le rendu
des pixels reste sur GPU ; aucun worker de rendu CPU n'est ajouté. Les solvers
CPU de diffusion et d'onde servent aussi de références indépendantes aux tests.

Les champs sont échantillonnés dans un domaine fini, interpolés trilinéairement,
puis transmis par le buffer/texture de paramètres existant. Le contrôle
précompilé et le rendu spécialisé lisent les mêmes valeurs. La borne globale des
dérivées de cette interpolation fournit un pas conservatif pour les isosurfaces
et les déplacements. La surface visée est celle de ce champ discret, avec la
tolérance d'intersection du raymarcher. Augmenter la résolution améliore sa finesse ;
le budget mémoire et le domaine restent explicites. La Julia quaternionique utilise
l'estimateur extérieur dérivé de l'orbite `0.5 * r * ln(r) / dr` et un signe
intérieur ; cet estimateur ne constitue pas une distance euclidienne exacte.

Le volume naturel intègre émission et absorption de Beer–Lambert dans le HDR
linéaire. Il suit les segments courbes lorsqu'une lentille est active. L'exposition,
le débruitage et le développement conservent leur place après l'accumulation.

Le film utilise les amplitudes complexes d'Airy et les Fresnel s/p à l'angle de la
microfacette. Son RGB est intégré sur 41 bandes de 380 à 790 nm avec la réponse
CIE 1931 approchée de Wyman et al., sous illumination d'énergie égale, puis
normalisé et limité au gamut. La dispersion des trajets est une approximation à
trois bandes (610, 550, 460 nm), pondérée sans biais pour ce modèle RGB ; elle ne
constitue pas un moteur de transport spectral continu ni une reconstruction du
spectre mesuré des illuminants.
Chaque groupe de trois échantillons utilise toutes les bandes, avec une
permutation propre au pixel et au germe. L'indice absolu préserve cette
stratification entre les dalles, les dispatches et les reprises. Un environnement
uniforme ne reçoit donc plus de bruit chromatique à la fin d'un groupe complet.

Les matériaux transmissifs utilisent aussi la couche mince : les probabilités de
réflexion et les poids de transmission restent complémentaires par canal. Les
indices du matériau définissent les deux sens de traversée ; une paroi mince
utilise de l'air de part et d'autre du film. Les cosinus complexes conservent la
propagation évanescente et la réflexion totale frustrée. Les surfaces de mer et
le paysage du portail gardent leur propre matériau.

Les rayons de caméra de Kerr remontent le temps vers les sources, en utilisant le Hamiltonien nul, les dérivées numériques de la
métrique et RK4. Le pas décroît près de l'horizon et de la géométrie. Les
intersections et le tangent final sont conservés entre les étapes wavefront ; le
pointage reconstruit également un trajet courbe. L'éclairage des matériaux reste
le modèle local du moteur : ni décalage gravitationnel de fréquence ni transfert
radiatif relativiste complet ne sont simulés. Les connexions d'éclairage direct
euclidiennes sont désactivées pour ces trajets. Brouillard, mer, nébuleuse et
portail historiques, ainsi que la diffusion sous-surfacique, sont refusés avec
Kerr plutôt que mélangés à une trajectoire incorrecte ; utiliser les volumes
naturels pour la matière traversée par les géodésiques.

L'export spécialisé élimine les branches de phénomènes absentes de la scène. Le
contrôle nodal conserve sa machine précompilée ; la spécialisation reste une
action explicite. Les paramètres numériques réutilisent les pipelines. Un cache
de deux préparations ignore caméra, développement, palette et gains de rendu ;
changer ces réglages ne relance pas la simulation.

Les résultats CPU/GPU sont comparés avec des tolérances mesurées. Aucune identité
binaire entre matériels ou pilotes différents n'est promise. Les états figés
préservent les valeurs stockées, et les signatures de reprise incluent le code
des nouveaux kernels et des préparateurs.

## CLI et reproduction

Depuis la racine du dépôt, après compilation :

```powershell
.\target\debug\syfrack.exe nature list
.\target\debug\syfrack.exe nature preset morphogenesis --out scenes\mon-recif.ron
.\target\debug\syfrack.exe nature inspect scenes\mon-recif.ron
.\target\debug\syfrack.exe nature freeze scenes\mon-recif.ron --out scenes\mon-recif-fige.ron
.\target\debug\syfrack.exe nature frame scenes\mon-recif.ron --time 2.5 --out scenes\mon-recif-t250.ron
.\target\debug\syfrack.exe nature variants scenes\mon-recif.ron --out renders\mes-variantes --parameters look.exposure_ev,camera.focal_mm --count 12 --seed 7
.\target\debug\syfrack.exe render scenes\mon-recif-fige.ron --out renders\mon-recif --width 1920 --height 1080 --samples 128 --backend wavefront
cargo run --locked -p syfrack-render --example nature_lab -- renders\nature-cosmos-20261019
```

Les commandes de création de scène et de variantes refusent d'écraser une sortie
existante. Les fichiers d'état d'export habituels restent utilisés.

La vérification complète se lance aussi avec :

```powershell
.\scripts\verify-nature.ps1 -Gpu -Gallery -Capture
```

Les validations automatisées couvrent le typage, les cycles, les états figés, la
chronologie, l'atlas, la conservation de masse d'une onde non amortie, la
convergence d'un mode propre, les bornes d'interpolation, les shaders Naga/SPIR-V,
la parité des sondes contrôle/spécialisé, les PDE sur GPU, les géodésiques et la
continuité référence/wavefront. Les tests Studio vérifient les deux langues,
l'enregistrement des connexions incomplètes et la préparation asynchrone.

## Références mathématiques

- [Pearson : motifs de réaction–diffusion](https://arxiv.org/abs/patt-sol/9304003).
- [Gross–Pitaevskii et vortex](https://arxiv.org/abs/1110.5757).
- [Croissance diélectrique de Niemeyer, Pietronero et Wiesmann](https://doi.org/10.1103/PhysRevLett.52.1033).
- [Belcour et Barla : couches minces](https://belcour.github.io/blog/research/publication/2017/05/01/brdf-thin-film.html).
- [Wyman, Sloan et Shirley : réponse CIE analytique, équation 4](https://jcgt.org/published/0002/02/01/).
- [Levy : modes du Laplacien](https://brunolevy.github.io/papers/Laplacian_SMI_2006.pdf).
- [James et al. : rendu de géodésiques autour de Kerr](https://arxiv.org/abs/1502.03808).
