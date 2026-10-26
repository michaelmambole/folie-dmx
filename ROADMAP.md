# Feuille de route Folie-DMX

Estimations pour un développement **à temps partiel** (soirs et week-ends).
Objectif : une console utilisable en spectacle en **6 à 9 mois**, puis enrichissement.

## Phase 1 — Noyau DMX ✅
- [x] Univers, trait de sortie, sortie virtuelle
- [x] Pilote Enttec USB Pro (trame label 6, paramètres, numéro de série)
- [x] Moteur à fréquence fixe, commandes, instantané, blackout
- [x] **Validation sur matériel réel** (2026-10-26, au gril d'une salle) : l'USB Pro exige 250 000 bauds
  et un débit limité (~25 trames/s) ; interface choisie automatiquement au lancement.
  Validés sur LEDBeam 350 (mode 2) et SupraKolor HD (TOUR) : dimmer, shutter/strobe, pan/tilt,
  couleurs RGBW et sélecteur, zoom, cue avec fondu, effet. Rush MH 6 : roue de couleurs simulée
  et sens du zoom (large à 0) ; couleurs blanches par défaut.
- [ ] Reste à essayer sur appareil : **Sunstrip** (rangés pour l'instant) ; les LEDBeam en 001 et 228 (autre perche, après un splitter dans les murs) ne répondent pas à l'USB Pro, ni avec Folie ni avec EMU, alors qu'elles répondent à la grandMA2 par le même câble : question d'installation, pas de Folie (MAB et break modifiés sans effet)
- [ ] Fluidité des fondus à 25 trames/s à surveiller sur des fondus lents de dimmer
- [x] **ServoColor 4K** vérifiée (dimmer, strobe dès 11, pan/tilt, zoom large à 0) ; sa roue virtuelle
  exige le Blanc à 0 (ses blancs 1 à 10 aussi RGB à fond) : Blanc à 0 par défaut, et remis à 0 tout
  seul dès qu'on utilise la roue (extension `folieTurnsOff` dans la fiche) ; défilement dès 251.
- [x] Reconnexion automatique si l'interface est débranchée (une tentative par seconde, alerte rouge jusqu'au retour)

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
- [x] Macros : pool (⌘0), éditeur avec vérification des lignes, `Wait`, `Macro 3`, `Store/Label/Delete Macro`, lancées par un executor
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
- [x] Fenêtre Scène (⌘9) : projecteurs en blocs, faisceaux en traits colorés, caméra, sélection au clic
- [x] Scène : placement des projecteurs à la souris, hauteur, accroche, plan enregistré dans le show
- [x] Réaction au son (crate `folie-audio`, fenêtre Son) : entrée audio (appareil, canal, gain),
      niveaux global / graves / médiums / aigus, beat et tempo détectés ; tempo du show qui suit
      le beat, pas de chenillard, fader au niveau, flash sur le beat (envoyés au moteur depuis le fil audio)
- [x] Logo « Folie » (le « l » et le « i » en jauges, point rouge), icône « li » de l'application et de la télécommande
- [x] Barre du haut en trois zones (show et menus, touches STORE / CLEAR / BLACKOUT, pastilles d'état),
      ligne de commande `Folie>`, barre d'état réduite à l'essentiel, fenêtre « À propos »
- [x] Accueil sans sélection (logo, prochaine étape, raccourcis des fenêtres et des touches),
      cases libres des pools en simple contour, touche Patcher plus visible
- [x] Executors vides discrets : numéro atténué, rail en pointillés et « vide », sans touches ni fader
- [x] Gélatines Lee et Rosco (environ 95 références courantes, teintes approchées) dans le sélecteur de couleur

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
- [x] **OSC** (crate `rosc`, UDP) : `/folie/exec/N/fader`, `/go`, `/back`, `/off`, `/flash`, `/folie/go`,
      `/folie/page`, `/folie/blackout`, `/folie/cmd "<commande>"`, avec retour d'état vers les clients
      (TouchOSC, QLab…) ; réglages dans le menu Télécommande

➡️ **Fin de phase 4 = console utilisable sur un vrai spectacle.**

## Phase 5 — Effets (≈ 1 à 1,5 mois)
- [x] Générateurs dans le moteur : sinus, rampes, carré, triangle ; phase continue
- [x] Éventail dans l'ordre de sélection, ailes en miroir, blocs
- [x] Vitesse en Hz ou calée sur le tempo du show ; Tap tempo, `Tempo 120`
- [x] Modèles : Sinus dimmer, Chenillard, Vague, Cercle, Arc-en-ciel, Pulse (`Effect n`)
- [x] Effets dans le programmer, enregistrés dans les cues, suivis par tracking
- [x] Fenêtre Effets (⌘7)
- [x] Arrêter un effet seul dans une cue, fondu d'entrée et de sortie des effets
- [x] Effets relatifs (autour de la valeur en cours, donc aussi d'un preset rappelé) ; Cercle relatif
- [x] Rapport cyclique du carré, forme d'onde aléatoire, modèle Scintillement
- [x] Pool d'effets personnalisés (Mes effets : Au pool, relancer, renommer, supprimer)
- [x] Effets visibles dans la fenêtre Programmer (pastilles, plage, nom de l'effet)

## Phase 6 — Réseau et télécommande (≈ 1,5 à 2 mois)
- [x] Sortie Art-Net (ArtDmx vers un node ou en diffusion), en plus de la sortie choisie
- [x] Sortie sACN (E1.31) : multicast ou node, univers 1 à 63999, priorité, en plus de la sortie choisie
- [ ] Univers multiples (l'USB Pro n'en gère qu'un), découverte des nodes (ArtPoll)
- MIDI pour piloter depuis une surface externe
- [x] **Télécommande web** (crate `folie-remote`) : serveur axum + WebSockets, page HTML/JS
      embarquée dans le binaire ; executors de la page (faders, Go/Back/Off, Flash maintenu,
      Bascule, macros), pages, Go/Back de l'executor choisi, blackout, ligne de commande avec
      pavé, groupes ; bouton « Télécommande » de la barre du haut (adresse, QR code, code PIN)
- [x] Toutes les adresses de l'ordinateur avec le nom de la connexion (Wi-Fi, Ethernet), choix de celle du QR code
- [x] Sécurité du web : coupé par défaut, deux codes PIN, régie (tout) et lecture (GO, Back,
      Flash, vérifié par le serveur), une adresse mise à l'écart une minute après
      5 erreurs), réseau local uniquement, « Couper la télécommande » ; Flash relâché si le
      téléphone se déconnecte
- [x] Envoi OSC : destinations nommées dans le show, commande `Osc "Reaper" "/play"`, commande
      jouée au départ d'une cue (« CMD »), destination Reaper toute prête
- [x] Marqueurs de Reaper : le marqueur N fait Goto Cue N (à vérifier avec un vrai Reaper)
- [x] OSC : réseau local uniquement, liste d'adresses autorisées, coupé par défaut

Un seul show et un seul programmer : plusieurs moyens de piloter, pas de multi-utilisateur.

## Plus tard (hors périmètre initial)
- Visualiseur 3D, multi-utilisateur, timecode, import de shows MA
