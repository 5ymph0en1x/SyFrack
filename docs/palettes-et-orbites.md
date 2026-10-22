# Palettes cycliques et coloration des orbites

Les palettes de la surface et de sa texture fractale partagent les mêmes
réglages. Les scènes existantes gardent leurs couleurs : les nouveaux champs
ont des valeurs par défaut correspondant à l'ancien dégradé linéaire.

## Palette

Une palette accepte de 2 à 16 couleurs RGB **linéaires**. Dans Studio, cliquer
dans la bande ajoute une couleur ; glisser horizontalement la place, glisser
verticalement ajuste sa luminosité, et la sortir de la bande la retire.
Les pastilles ouvrent le sélecteur de couleur. « Espacer régulièrement »
rétablit des positions uniformes.

- **Palette cyclique** referme le dernier segment sur la première couleur.
- **Fréquence** règle le nombre de répétitions sur une unité du signal.
- **Phase** fait défiler la palette, y compris dans le sens négatif.
- **Gamma** remappe la coordonnée dans chaque cycle.
- **Transitions douces** utilise une interpolation smoothstep entre couleurs.

Les positions explicites sont strictement croissantes. Elles appartiennent
à [0, 1] pour un dégradé ouvert, et à [0, 1[ pour une palette cyclique.
Sans positions explicites, une palette ouverte espace les couleurs sur
0…1 ; une palette cyclique les espace sur 0…(n−1)/n pour réserver le raccord.

```ron
ramp: (
    stops: [(0.01, 0.03, 0.12), (0.02, 0.8, 0.9), (0.95, 0.12, 0.04)],
    positions: [0.0, 0.35, 0.8],
    cyclic: true,
    frequency: 13.0,
    phase: -0.2,
    gamma: 1.0,
    interpolation: Smooth,
)
```

## Mesures d'orbite

Le nœud **Piège** propose les distances existantes, les angles XY/XZ/YZ
(atan2 ramené dans [0, 1]), et une norme logarithmique log(1 + |p|).
L'angle du point nul vaut 0,5. Chaque mesure peut enregistrer le minimum,
le maximum ou la dernière valeur rencontrée. Elle est ensuite divisée par
son échelle et bornée à 1, comme les anciens pièges. Ces mesures observent
l'orbite sans modifier la géométrie.

Le langage Forme expose ces choix :

```text
angle = mesurer(; canal=2; mesure=angle_xy; accumulation=dernière; échelle=1)
```

**Mélange des orbites** combine les quatre pièges, le compte d'itérations
normalisé et la normale Y de la surface, avec des poids signés. Un étalement
logarithmique optionnel répartit les variations concentrées. Le résultat
reste signé et peut dépasser 1 : c'est la palette qui le répète ou le borne.
Dans une texture volumique indépendante, la normale n'est pas disponible ;
son poids fournit un décalage constant égal à la moitié du poids.

```ron
coloring: Orbit(
    channels: (
        traps: (0.6, -0.2, 0.3, 0.1),
        iterations: 0.15,
        normal: 0.0,
        logarithmic: 2.0,
    ),
    ramp: (stops: [(0.0, 0.1, 0.3), (0.1, 0.8, 0.9)], cyclic: true),
)
```

Dans `material.texture`, le même mélange s'écrit `channel: Orbit(channels: (...))`.
Dans les textures par graphe, le compte d'itérations reste discret. Le mode
facultatif **Fractale complexe** apporte six familles 2D et des canaux dédiés,
dont un temps d'échappement lissé. Voir
[les équations et le laboratoire esthétique](fractales-complexes-et-recherche-esthetique.md).

## Première composition

`python scripts/compose/compose-volutes-chromatiques.py` crée une étude
reproductible avec treize inversions récursives et une palette de seize
couleurs. Le support est un plan ; les volutes sont une texture mathématique
native, avec un relief de normales filtré. Elles ne constituent pas une
géométrie volumique ni une reproduction de la formule de l'image de référence.

L'extension change les shaders et la disposition des uniformes GPU. Les
anciens projets RON restent lisibles ; les anciens checkpoints de rendu se
reprennent avec leur exécutable d'origine, conservé pour cette mise à jour
dans `target/laptop-before-orbits-20261013/`.
