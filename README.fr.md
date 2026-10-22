# SyFrack Studio

**L’art fractal, du plan complexe aux mondes monumentaux.**

[English](README.md) · [Guide d’utilisation en anglais](docs/USER_GUIDE.md)

SyFrack est un studio expérimental de création et de rendu de fractales écrit
en Rust. Un seul graphe de création combine formes, motifs, flammes GPU et
champs naturels. **2D / 3D** change la projection du même document, avec un
historique, un aperçu et un export communs. Voir le [graphe unifié](docs/graphe-unifie.md).

## Choisir son interface

La barre de navigation propose **Simple / Avancée** à tout moment. Le mode
**Simple** organise les réglages essentiels : Motif, Couleurs et Cadrage en 2D ;
Volume, Matière, Lumières et Image en 3D. Le mode **Avancée**, choisi par défaut
au premier lancement, conserve l’accès direct au graphe complet, à la mosaïque
et à la galerie.

Les deux interfaces modifient le même document. Le changement conserve la scène,
la sélection du graphe et l’historique d’annulation ; le choix est mémorisé.
Le menu **Outils** du mode simple permet de rejoindre la mosaïque ou le graphe.

![Le Souffle des Titans — rendu natif de SyFrack](docs/images/breath-of-the-titans.jpg)

## Langue de l’interface

L’anglais est utilisé au premier lancement. Choisissez **View → Language →
Français** pour passer au français, ou **Affichage → Langue → English** pour
revenir à l’anglais. Le choix est mémorisé ; il ne modifie ni la scène ni son
historique. Les champs numériques acceptent le point et la virgule.

Les titres personnels, les chemins et les projets existants conservent leur nom.
Les anciens mots-clés français du langage Forme restent acceptés ; le code
exporté utilise désormais un vocabulaire anglais stable.

![Atelier 3D en français](docs/images/studio-fr.png)

## Télécharger pour Windows

Dans la page **Releases** du dépôt, téléchargez l’archive
**SyFrack-v…-windows-x64.zip**. Extrayez tout son contenu dans un dossier
accessible en écriture, puis lancez **syfrack-studio.exe**. Studio, la CLI, les
scènes, les graphes et les assets HDRI sont inclus. Rust et Python ne sont pas
nécessaires pour utiliser les exécutables. Conservez `graphs/`, `scenes/` et
`assets/` à côté des exécutables.

Studio nécessite Windows x64 et un GPU/pilote compatible Vulkan ou Direct3D 12.
Ces binaires expérimentaux ne sont pas signés. `SHA256SUMS.txt` permet de vérifier
l’intégrité du ZIP ; il ne constitue pas une signature. Le runtime OIDN reste
facultatif et n’est pas inclus.

## Compiler et démarrer

Windows x64 est la plateforme principalement testée. Installez Rust ainsi que
les outils C++ MSVC. La version de Rust est fixée dans `rust-toolchain.toml`.
Studio utilise Vulkan ou Direct3D 12. Le moteur CLI essaie aussi OpenGL en dernier recours.

À la racine du dossier :

```powershell
cargo build --release --locked --workspace --bins
cargo run --release --locked -p syfrack-cli -- probe
cargo run --release --locked -p syfrack-app --bin syfrack-studio
```

Un script est également fourni :

```powershell
powershell -ExecutionPolicy Bypass -File scripts/build.ps1
```

## Créer et exporter

En 2D, explorez le motif, les couleurs et le cadrage. En 3D, façonnez le volume,
réglez sa matière, placez les sources et développez l’image. **Graphe** donne
accès aux sources, aux connexions et aux outils spécialisés.

**Rendre / Exporter** produit un PNG 16 bits, un EXR linéaire brut et un instantané
RON du projet. Les réglages personnalisés comprennent les échantillons, le
nombre de rebonds, la précision géométrique et les limites de calcul. Les rendus
peuvent être mis en pause puis repris depuis leur checkpoint.

La fenêtre de rendu distingue le format, la qualité et la destination. Elle
présente les noms exacts des fichiers, garde le lancement et la pause accessibles
pendant le défilement, et regroupe les paramètres techniques dans les réglages
personnalisés. L’aperçu terminé reçoit bien les dernières tuiles et le débruitage
éventuel du PNG.

![La fenêtre de rendu et d’export](docs/images/studio-render-fr.png)

Commencez avec une définition modeste. Les volumes récursifs, les matières
transmissives et les nombreux rebonds peuvent demander un calcul long.

## Version expérimentale

L'onglet **MOSAÏQUE** permet de [réinterpréter une image avec des fractales et
palettes locales](docs/mosaique-fractale.md), avec recadrage, réglages de couleur,
découpage adaptatif et comparaison. L'outil reste disponible dans la CLI.
Les recettes exportées se reconstruisent sans l'image source.

Cette version **0.2.0** reste un logiciel en évolution. La compatibilité GPU et
les performances dépendent du matériel. Certains effets volumétriques et le
guidage de lumière indirecte sont expérimentaux. Un checkpoint dépend aussi de
la version du moteur : conservez le binaire correspondant pour une reprise
future. L’EXR brut reste disponible même si le PNG est débruité.
Le [journal des changements](CHANGELOG.md) décrit les évolutions depuis la
préparation publique initiale.

Les sources sont sous [licence MIT](LICENSE). La provenance des panoramas HDRI
et le moteur de débruitage facultatif sont décrits dans
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
