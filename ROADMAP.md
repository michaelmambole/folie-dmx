# Feuille de route Folie-DMX

Estimations pour un développement **à temps partiel** (soirs et week-ends).
Objectif : une console utilisable en spectacle en **6 à 9 mois**, puis enrichissement.

## Phase 1 — Noyau DMX ✅ (en cours de validation)
- [x] Univers, trait de sortie, sortie virtuelle
- [x] Pilote Enttec USB Pro (trame label 6, paramètres, numéro de série)
- [x] Moteur à fréquence fixe, commandes, instantané, blackout
- [ ] **Validation sur matériel réel** (`folie info`, `folie test`)
- [ ] Reconnexion automatique si l'interface est débranchée

## Phase 2 — Interface et patch (≈ 1 à 1,5 mois)
- [x] Interface graphique **egui** : moniteur 512 canaux, banque de 16 faders par page,
      choix de la sortie à chaud, blackout (touche B), affichage % / DMX
- [x] Import de fixtures au format Open Fixture Library (dossier `library/`)
- [x] Fiche Martin Rush MH 6 Wash (14 canaux) écrite d'après le manuel constructeur
- [x] Patch : numéro de fixture, mode, adresse automatique, contrôle des chevauchements
- [x] Attributs normalisés regroupés par famille (Dimmer, Position, Couleur…), 16 bits
- [x] Vue Fixtures : sélection multiple, réglage par attribut, menus de plages
      (couleurs avec pastilles, strobe…), angles physiques
- [x] Multi-instance : matrices OFL (1D), sous-fixtures « 5.1 … 5.10 » dans le patch et la sélection
- [x] Fiches Robe LEDBeam 350 (3 modes), Starway SupraKolor HD (12 ch), Sunstrip (OFL)
- [x] Starway ServoColor 4K (OFL), avec canaux commutés (fonction par défaut seulement)
- [ ] Canaux commutés complets : fonction qui suit la valeur du canal de commande
- [x] Tests d'interface automatisés (egui_kittest)
- [x] Fichier de show `.folie` (JSON versionné, fiches embarquées), écriture atomique
- [x] Sauvegarde automatique toutes les 30 s, récupération après plantage,
      confirmation avant de perdre des modifications, réouverture du dernier show
- [ ] Inversion pan/tilt, univers multiples
- [ ] Import GDTF

## Phase 3 — Programmer (≈ 1,5 mois)
- [x] Couche de contrôle `folie-control` : sélection ordonnée, groupes, presets, commandes
- [x] Groupes (ordre de sélection conservé), pool cliquable, enregistrés dans le show
- [x] Presets par famille numérotée comme grandMA2 (1 Dimmer … 7 Contrôle), pool cliquable
- [x] Ligne de commande style MA avec abréviations et historique :
      `Fixture 1 Thru 4 - 2 At 50`, `Group 2 At Full`, `Fixture 5.1 Thru 5.4`, `Preset 4.1`,
      `Store Group 3 "Cour"`, `Store Preset 4.1`, `Label`, `Delete`, `Clear`, `ClearAll`
- [x] Presets référencés (et non recopiés) : faits avec les cues, phase 4
- [x] Commandes de valeur par attribut (`Attribute "Pan" At 50`), `At` relatif (`At + 10`, `At - 5`)
- **Couche de contrôle** entre les interfaces et le moteur : elle tient le show et le
  programmer, et le langage de commande en est l'**unique point d'entrée**
  (egui, OSC et web passent tous par elle)
- Sauvegarde et chargement du show (format texte versionnable, ex. RON/JSON)

## Interface : fenêtres à la manière d'une station audio
- [x] Fenêtre principale « Programmer » (attributs, ligne de commande, état)
- [x] Fenêtres système séparées : Patch ⌘1, Groupes ⌘2, Presets ⌘3, Sortie DMX ⌘4
- [x] Disposition de départ calculée selon l'écran, espace de travail retrouvé au lancement
- [x] Dispositions enregistrées (cases 1 à 9), rappel ⌥⌘1 à ⌥⌘9, mise à jour, suppression
- [x] Fenêtre ramenée sur l'écran principal si son écran n'est plus branché
- [ ] Plusieurs fenêtres d'un même type (ex. deux pools de presets)
- [ ] Dispositions embarquées dans le show

## Identité visuelle et ergonomie
- [x] Thème : palette à sens fixe, polices Inter et JetBrains Mono, cartes et sections
- [x] Programmer : barres de valeur colorées selon la source, Option-clic pour relâcher
- [x] Sélecteur de couleur traduit par appareil (RGB, RGBW, CMY, roue seule), couleurs rapides
- [x] Executors : fader dessiné, cartes adaptatives ; pools : couleur obtenue, LED d'état
- [x] Fenêtres Cues, Patch, Effets et Sortie DMX au même niveau visuel
- [ ] Gélatines (Lee, Rosco) dans les couleurs rapides

## Phase 4 — Cues et executors (≈ 2 à 3 mois, le cœur)
- [x] Séquences de cues numérotées au millième (1, 1.5…), fondu et délai par cue
- [x] **Tracking** calculé par la console, `Release` pour rendre un paramètre
- [x] Presets référencés dans les cues : modifier le preset modifie les cues
- [x] Moteur : fondus à 44 Hz, executors en HTP (intensité × fader) et LTP (dernier lancé)
- [x] Commandes `Go`, `Back`, `Off`, `Goto Cue`, `Executor n At`, `Store Cue n Fade s "nom"`
- [x] Fenêtres Cues (⌘5) et Executors (⌘6)
- [x] Sauvegarde automatique, reprise après plantage
- [x] `Block` et `Unblock` (figer les valeurs suivies), repérés par un « B » dans la fenêtre Tracking
- [x] Fondu des attributs LTP depuis et vers ce qui est dessous (valeur par défaut ou executor inférieur)
- [x] Executors en modes Go, Flash et Bascule ; 10 à fader et 10 à boutons
- [x] Touche Store globale : Store puis clic sur un executor, une case de pool ou une cue
- [x] Executors toujours complets (cue en cours et suivante, progression), menu au clic droit
- [x] Options d'executor : bouclage, Auto start/stop, fader Master/Temp/Vitesse, temps d'Off
      et de Back, priorité basse/normale/haute, protection ; « Tout éteindre » (`Off All`)
- [x] Pools : renommer et supprimer une case au clic droit
- [x] Pages d'executors (`Page 2`, `Page +`, `Page -`, `Go Executor 2.5`), pages nommées,
      executors fixés sur toutes les pages (`Fix Executor 3`)
- [x] Fenêtre Tracking (⌘8) : valeurs de chaque cue, changées, suivies, relâchées, presets, effets
- [x] Temps par famille (fenêtre Cues) et par paramètre (fenêtre Tracking), fondu et délai propres
- [x] Mode chenillard (BPM ou tempo du show, fondu entre pas, sens), fader Crossfade A/B, reprise, Ooo, MIB, Swop, Kill, Assert, Release
- **OSC** (crate `rosc`, UDP) : `/folie/exec/N/fader`, `/folie/exec/N/go`, `/folie/exec/N/back`,
  `/folie/cmd "<commande>"`, avec retour d'état vers les clients (TouchOSC, QLab…)

➡️ **Fin de phase 4 = console utilisable sur un vrai spectacle.**

## Phase 5 — Effets (≈ 1 à 1,5 mois)
- [x] Générateurs dans le moteur : sinus, rampes, carré, triangle ; phase continue
- [x] Éventail dans l'ordre de sélection, ailes en miroir, blocs
- [x] Vitesse en Hz ou calée sur le tempo du show ; Tap tempo, `Tempo 120`
- [x] Modèles : Sinus dimmer, Chenillard, Vague, Cercle, Arc-en-ciel, Pulse (`Effect n`)
- [x] Effets dans le programmer, enregistrés dans les cues, suivis par tracking
- [x] Fenêtre Effets (⌘7)
- [ ] Arrêter un effet seul dans une cue, fondu d'entrée et de sortie des effets
- [ ] Effets relatifs (autour de la valeur en cours) et sur presets
- [ ] Pool d'effets personnalisés, rapport cyclique du carré, aléatoire
- [ ] Effets visibles dans la fenêtre Programmer

## Phase 6 — Réseau et télécommande (≈ 1,5 à 2 mois)
- Art-Net et sACN (univers multiples ; l'USB Pro n'en gère qu'un)
- MIDI pour piloter depuis une surface externe
- **Télécommande web** : serveur axum + WebSockets, page embarquée dans le binaire
  (HTML/JS simple ou Leptos, sans Node) : executors, Go/Back, fixtures
- Sécurité des accès distants : web désactivé par défaut + code PIN, liste d'IP autorisées
  pour l'OSC, réseau local uniquement, bouton « Remote off » sur la console

Un seul show et un seul programmer : plusieurs moyens de piloter, pas de multi-utilisateur.

## Plus tard (hors périmètre initial)
- Visualiseur 3D, multi-utilisateur, timecode, import de shows MA
