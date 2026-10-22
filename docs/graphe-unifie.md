# Un graphe de création

Le Studio utilise un seul document `creation`, un aperçu GPU commun et les mêmes champs pour l’export. Les anciens fichiers sont migrés à leur ouverture. Les graphes de distance et de champs produits par le compilateur sont des données d’exécution : ils ne sont plus enregistrés comme des compositions indépendantes.

L’[atelier nodal révisé](atelier-nodal.md) propose désormais un parcours des
étapes existantes, des connexions guidées dans les deux sens, un inspecteur
de raccordement, une insertion dans les chaînes et des constructions complètes.
L’onglet **Ajouter** de la bibliothèque donne accès aux sources et traitements.

Dans **Graphe**, ajoutez des sources depuis la bibliothèque, puis reliez les prises de même couleur. Un motif fournit une valeur à une palette ; une flamme fournit une couleur et une densité ; une simulation reçoit éventuellement un potentiel, une source et un masque issus des autres nœuds. Les opérations Somme, Produit, Mélanger et Multiplier par une valeur permettent leurs combinaisons. Une seule sortie **Scène** réunit géométrie, couleur, émission, densité, déplacement, nacre, isosurface et texture de surface.

Les états de simulation disposent de canaux directs et d’une sortie État. Le nœud **Échantillonner l’état** permet de lire un canal à une autre position, sans relancer une simulation. Les boucles géométriques conservent leurs étapes ordonnées et leur sémantique de désactivation. Les expressions de distance natives restent inchangées.

Le choix 2D/3D change la projection de cette même scène ; le fichier et l’historique sont communs. Une sortie couleur est nécessaire en 2D ; une géométrie ou une isosurface est nécessaire en 3D. Les paramètres de cadrage, matériau physique et développement restent dans leurs inspecteurs. L’interface **Simple** donne accès aux réglages artistiques des sources et palettes raccordées. L’interface **Avancée** conserve tous les réglages et toutes les connexions du même graphe. Passer de l’une à l’autre conserve la scène, la sélection et l’historique. En mode simple, choisir 2D depuis une scène sans sortie couleur prépare un motif et une palette éditables, en conservant la géométrie ; une annulation rétablit la scène précédente.

Une connexion incomplète devient un brouillon enregistré. L’aperçu et l’export utilisent le dernier graphe valide. Les types incompatibles et les cycles sont refusés. Supprimer une liaison se fait par clic droit sur sa prise d’entrée. Ctrl+A sélectionne les cartes, Ctrl+D les duplique, Suppr les retire. Glisser le titre d’une boucle déplace aussi ses étapes. Le bouton Organiser dispose les cartes, Tout cadrer ajuste la vue.

Le contrôle GPU est précompilé : modifier les connexions charge des instructions et paramètres, sans reconstruire un shader spécialisé. La compilation spécialisée reste accessible dans le mini-aperçu. Le contrôle 3D emploie un éclairage simplifié ; le rendu final conserve le transport physique. Les motifs analytiques sont évalués aux coordonnées des échantillons, sans réduction préalable de leurs détails en grille. Les simulations, dérivées spatiales et déplacements utilisent un domaine fini explicite, configurable sur Scène. Ces champs ont la précision de leur discrétisation ; aucune identité mathématique avec une solution continue n’est promise.

Les flammes sont des sources d’orbites calculées sur le GPU, puis échantillonnées comme les autres champs. Leur résolution et leur budget de points sont indépendants de la taille de l’image. Les sommes de densité et de couleur utilisent des compteurs positifs de 64 bits composés de deux atomiques de 32 bits. Le budget mémoire est vérifié avant préparation. Les sources réutilisables sont mises en cache.

**Figer les sources GPU** conserve les atlas et états exacts dans le graphe, avec annulation et priorité au rendu final. Le fichier devient autonome pour ces données. Libérer l’état figé permet de modifier à nouveau sa génération. Animation, guides et atlas de variantes sont accessibles dans l’inspecteur Scène ; la source RON de ce nœud donne accès à tous les réglages du graphe.

Les checkpoints enregistrent ce même document et les accumulateurs. Reprise, rendu résident et rendu par dalles partagent les indices absolus des échantillons et le développement global de l’image HDR. Le contrôle à un échantillon, l’antialiasing final et les différences flottantes entre matériels restent des sources normales de différences visuelles.
