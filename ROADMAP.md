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
- Sélection de fixtures, groupes, valeurs par attribut
- Presets (position, couleur, faisceau…) référencés, pas copiés
- Ligne de commande style MA : `Fixture 1 Thru 10 At 50`, `Group 2 At Full`, `Clear`
- **Couche de contrôle** entre les interfaces et le moteur : elle tient le show et le
  programmer, et le langage de commande en est l'**unique point d'entrée**
  (egui, OSC et web passent tous par elle)
- Sauvegarde et chargement du show (format texte versionnable, ex. RON/JSON)

## Phase 4 — Cues et executors (≈ 2 à 3 mois, le cœur)
- Listes de cues, Go / Back / Goto, temps de fondu et de délai
- **Tracking** et cues bloquées (`Block`)
- Executors sur faders, priorités, fusion HTP / LTP
- Sauvegarde automatique, reprise après plantage
- **OSC** (crate `rosc`, UDP) : `/folie/exec/N/fader`, `/folie/exec/N/go`, `/folie/exec/N/back`,
  `/folie/cmd "<commande>"`, avec retour d'état vers les clients (TouchOSC, QLab…)

➡️ **Fin de phase 4 = console utilisable sur un vrai spectacle.**

## Phase 5 — Effets (≈ 1 à 1,5 mois)
- Générateurs (sinus, rampe, carré…), phase, ailes (wings), groupes
- Effets sur presets, vitesse liée à un tap tempo

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
