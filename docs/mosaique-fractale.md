# Mosaïque fractale adaptative

Cet essai réinterprète une image en régions de tailles variables. Chaque région
contient une fractale 2D, son cadrage, son orientation, un canal d'orbite et une
palette de quatre couleurs. La recette RON se reconstruit sans l'image source.
Le calcul s'effectue sur le CPU, dans l'onglet **MOSAÏQUE** de Studio ou depuis
la CLI. L'atelier mosaïque conserve ses images et réglages quand on change
d'onglet ; ses modifications ont leur propre historique.

## Dans Studio

1. Ouvrir l'onglet **MOSAÏQUE**, puis **Ouvrir une image / un projet…**.
   Le sélecteur accepte PNG, JPEG, BMP, TIFF et les projets `.mosaic-project.ron`.
2. Ajuster l'exposition, le contraste et la saturation. **Recadrer à la souris**
   permet de tracer la région à conserver sur l'image originale ; **+90°**
   applique une rotation après le recadrage. **Image entière** annule le
   recadrage ; **Réinitialiser l'image** remet aussi les couleurs et la rotation.
3. Choisir **Rapide**, **Équilibré** ou **Détaillé**, puis affiner la définition,
   la finesse, les tailles des régions et le budget. Les vues **Image ajustée**
   et **Découpage** s'actualisent en arrière-plan après une courte pause.
   Dans **Préserver les détails**, **Raffinement par erreur** réserve une partie
   du budget aux régions que les fractales restituent mal (100 par défaut ;
   0 retrouve le découpage basé sur la source). **Peindre** donne une priorité
   supplémentaire aux yeux, aux contours ou à tout autre détail choisi.
   **Effacer** retire localement cette priorité ; **Tout effacer** remet la carte
   à zéro. Le rayon est exprimé en pourcentage du petit côté de l'image cadrée.
   Un trait continu s'annule avec **Ctrl+Z**. Les marques suivent l'image lors
   d'un recadrage ou d'une rotation et sont enregistrées dans le projet.
4. Cliquer **Générer / Actualiser** ou presser **Ctrl+Entrée**. La progression
   distingue le découpage, la recherche des fractales, le raffinement et la reconstruction.
   **Annuler** interrompt le calcul et conserve le résultat précédent.
5. Examiner **Mosaïque** ou **Comparer**. Déplacer le curseur **Avant / après**,
   zoomer avec la molette et glisser pour déplacer l'image. Le double-clic ou
   **Ajuster à la vue** restaure le cadrage de l'aperçu.
6. Dans **Destination et agrandissement**, choisir le nom, le dossier et le
   facteur d'export. **Exporter la mosaïque…** ou **Ctrl+S** produit un PNG
   16 bits, la carte du découpage, les recettes et un projet réouvrable.

Quand un réglage change, le résultat précédent est signalé et l'export attend
une nouvelle génération. **Ctrl+Z / Ctrl+Y** agit sur les réglages de cet atelier.
Le changement de langue conserve les paramètres et les images.
La vue **Découpage** est une estimation rapide avant la génération ; après
génération elle montre les limites réellement retenues. Le pinceau augmente le
poids des zones peintes dans le budget existant ; la taille minimale des régions
reste respectée. Pour des traits très fins, diminuer **Petites régions**.

L'export de Studio contient six fichiers portant le même préfixe : `.png`,
`.layout.png`, `.source.png`, `.original.png`, `.mosaic.ron` et
`.mosaic-project.ron`. Le projet référence la copie `.original.png` par un chemin
relatif : garder ces fichiers ensemble pour reprendre le travail sur une autre
machine. Cette copie de travail mesure au plus 2 048 pixels sur son grand côté.
L'image d'origine sur disque reste intacte. Choisir un autre nom pour exporter
une variante ; les fichiers existants ne sont pas remplacés.

## Essayer en ligne de commande

Depuis la racine du dépôt, dans PowerShell :

```powershell
cargo run --release --locked -p syfrack-cli -- mosaic build docs/images/breath-of-the-titans.png --out renders/mosaic/titans --size 768 --min-size 8 --max-size 128 --regions 4096 --detail 0.075 --scale 2
```

Pour une photo personnelle, remplacer le chemin d'entrée par celui d'un PNG,
JPEG, BMP ou TIFF. Les couleurs sont interprétées comme du sRGB ; les profils
ICC arbitraires ne sont pas convertis. L'orientation EXIF est appliquée.

Quatre fichiers sont produits :

- `titans.png` : la reconstruction par fractales, en PNG 16 bits ;
- `titans.layout.png` : l'image d'analyse et les limites des régions ;
- `titans.source.png` : l'image source ramenée à la résolution d'analyse ;
- `titans.mosaic.ron` : les recettes des régions, sans pixels source.

Les chemins de sortie existants sont refusés. Choisir un nouveau préfixe pour
une variante. Les rendus restent dans `renders/`, hors du suivi Git.

## Régler le découpage

Le découpage commence par l'image entière. Un arbre à quatre enfants subdivise
les régions selon leurs variations de couleur et de transparence. Les bandes
étroites peuvent être divisées en deux ; les dimensions impaires sont conservées
exactement. Chaque pixel appartient à une seule région.

Les régions les plus grandes dépassant `--max-size` sont divisées en priorité,
puis celles dont la variation totale est la plus forte. Tous les pixels
contribuent à cette mesure, y compris les détails fins. Les couleurs cachées par
une transparence nulle ne consomment pas de régions.

Le raffinement est activé par défaut dans Studio et la CLI. Une partie du budget
est réservée après ce premier découpage : les régions sont ajustées, puis leur
reconstruction est comparée à la source à la résolution d'analyse, avec les
quatre échantillons par pixel du rendu final. Les divisions qui réduisent l'erreur mesurée sont
conservées, ainsi que celles nécessaires pour atteindre la taille souhaitée.
Le budget et la taille minimale restent stricts. Les zones peintes peuvent
peser jusqu'à seize fois plus dans cette recherche.
Le résultat est aussi comparé au découpage d'origine au même budget : celui-ci
est conservé si sa mesure globale est meilleure. Avec le pinceau, cette mesure
est pondérée par les priorités. Cette garantie concerne la résolution d'analyse.

- `--detail` : seuil de variation perceptuelle, entre 0 et 1, excluant 0.
  Une valeur plus basse conserve davantage de détails.
- `--refinement` : intensité du raffinement de 0 à 1, 1 par défaut.
  À 0, le découpage repose uniquement sur les variations de la source.
- `--min-size` : taille minimale des enfants, en pixels d'analyse.
- `--max-size` : taille maximale souhaitée des régions.
- `--regions` : budget strict, de 1 à 16 384 régions. Il peut empêcher
  d'atteindre la taille maximale souhaitée ou le seuil de détail.
- `--size` : plus grand côté de l'analyse, 1 024 par défaut, jusqu'à 2 048.
  Les petites images ne sont pas agrandies avant l'analyse.

La subdivision dépend de l'image. Un ciel uni conserve de grandes régions ; les
textures, contours et changements de teintes reçoivent des régions plus fines.

## Choisir les fractales

Un atlas de 216 candidats combine six familles natives : Julia, Mandelbrot,
Burning Ship, Newton, Nova et Phoenix. Il explore trois cadrages ou variantes,
quatre orientations et trois canaux d'orbite. Une grille de 8 × 8 moyennes locales
décrit chaque région. Pour chaque candidat, une régression ajuste les quatre
couleurs ; l'erreur de reconstruction choisit le motif final.
Le raffinement dispose aussi de motifs intégrés selon la taille réelle des
régions, pour que les petites tuiles soient ajustées au signal anticrénelé rendu.

L'ajustement et le rééchantillonnage utilisent des couleurs linéaires et une
transparence prémultipliée. L'export restitue un PNG sRGB à alpha droit. Le rendu
évalue réellement les orbites, avec quatre échantillons par pixel.

Il s'agit d'une réinterprétation approchée, avec des raccords visibles entre
régions. L'atlas est fini. Le raffinement mesure l'erreur de tous les pixels à
la résolution d'analyse ; il ne garantit pas une amélioration de chaque pixel
ou le même classement des variantes après agrandissement. Le champ `error` de chaque
recette reste l'ajustement sur la grille de 8 × 8, pas l'erreur plein format.
Cette version ne promet ni restitution exacte, ni compression de l'image.

## Agrandir à partir des seules recettes

```powershell
cargo run --release --locked -p syfrack-cli -- mosaic render renders/mosaic/titans.mosaic.ron --out renders/mosaic/titans-x2.png --scale 2
```

Le fichier d'origine n'est pas ouvert. Les coordonnées locales des fractales
restent identiques ; le rendu calcule de nouveaux détails à la résolution
demandée. `--scale` est entier et la sortie est limitée à 16 mégapixels. La
colorimétrie et les formes globales restent celles ajustées lors de l'analyse.

## Vérification ciblée

```powershell
cargo test --locked -p syfrack-render --lib mosaic::
cargo test --locked -p syfrack-app --lib ui::mosaic::
cargo test --locked -p syfrack-cli --test mosaic_cli
```

Les tests vérifient la couverture exacte, les bords impairs, les bandes d'un
pixel, le budget, la concentration des régions sur les détails, la transparence,
l'ajustement structurel, l'anticrénelage des petites régions, la mesure du rendu,
la répartition des priorités, l'historique du pinceau, le suivi du recadrage et
de la rotation, la validation des recettes et la reconstruction après suppression
de l'image source.
