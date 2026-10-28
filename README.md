# Folie-DMX

Logiciel de contrôle lumière inspiré des standards de l'industrie, écrit en Rust,
avec sortie sur Enttec DMX USB Pro.

## Structure

| Crate | Rôle |
|---|---|
| `crates/folie-dmx` | Types DMX (`Universe`), trait `DmxOutput`, Enttec USB Pro, Art-Net, sACN |
| `crates/folie-fixture` | Modèle de fixtures, import OFL, patch, rendu attributs → DMX |
| `crates/folie-show` | Fichier de show `.folie` : lecture, écriture atomique |
| `crates/folie-control` | Console : sélection, groupes, presets, ligne de commande |
| `crates/folie-audio` | Réaction au son : entrée audio, niveaux par bande, beat, tempo |
| `crates/folie-remote` | Télécommande web (axum, WebSocket, page embarquée) et OSC |
| `crates/folie-engine` | Moteur temps réel : thread dédié à ~44 Hz, commandes par canal, état publié |
| `crates/folie-cli` | Binaire `folie` : diagnostic et tests matériels |
| `crates/folie-app` | Binaire `folie-dmx` : interface graphique (egui) |

Principe clé : **le moteur ne dépend jamais de l'interface**. Il reçoit des commandes
et publie un instantané de son état ; une interface lente ne peut pas faire
saccader la sortie DMX.

## Essayer

Rust 1.95 minimum (`rustup update`).

```bash
cargo run -p folie-app                               # interface graphique

cargo run -- ports                                   # lister les ports (FTDI en premier)
cargo run -- test                                    # sortie virtuelle, canaux 1-4
cargo run -- info --port /dev/cu.usbserial-XXXX      # interroger l'USB Pro
cargo run -- test --port /dev/cu.usbserial-XXXX --channels 1-8 --seconds 20
```

Les shows sont des fichiers `.folie` (JSON lisible) qui embarquent les fiches
de fixtures utilisées. Réglages et sauvegarde automatique :
`~/Library/Application Support/Folie-DMX/`.

La bibliothèque de fixtures est dans [library/](library/README.md) (format OFL).
Un autre dossier peut être utilisé avec la variable `FOLIE_LIBRARY`.

## Télécommande et OSC

Bouton **Télécommande** de la barre du haut ; tout est coupé tant qu'on ne l'active pas,
et seul le réseau local est accepté.

- **Web** : le téléphone ouvre l'adresse affichée (ou scanne le QR code) et donne un code :
  *régie* (tout) ou *lecture* (GO, Back, Flash). Sans l'interface :
  `cargo run -p folie-remote --example demo`, puis http://localhost:8090 (codes 1234 et 5678).
- **OSC** (UDP, port 8000 par défaut, retour d'état vers le port 9000 du client) :

| Adresse | Effet |
|---|---|
| `/folie/exec/3/fader 0.75` | fader de la place 3 de la page en cours (0 à 1) ; `2.5` : page 2, place 5 |
| `/folie/exec/3/go`, `back`, `off`, `press` | touches de l'executor (une valeur 0, le relâchement, est ignorée) |
| `/folie/exec/3/flash 1` / `0` | Flash enfoncé / relâché |
| `/folie/go`, `/folie/back` | executor sélectionné |
| `/folie/page 2`, `/folie/page/next`, `/folie/page/prev` | page d'executors |
| `/folie/blackout 1` / `0` (sans valeur : bascule), `/folie/offall`, `/folie/group 3` | |
| `/folie/cmd "Fixture 1 At 50"` | une ligne de commande |

Retour d'état : `/folie/exec/N/fader`, `/name`, `/cue`, `/on` (N : place 1 à 20 de la page
en cours), `/folie/page`, `/folie/page/name`, `/folie/blackout`.

### Reaper

**Folie pilote Reaper** : dans le menu Télécommande, « Ajouter Reaper » crée la destination
`127.0.0.1:8000` (enregistrée dans le show). Clic droit sur une cue, champ *Au départ* :

```text
Osc "Reaper" "/marker" 3; Osc "Reaper" "/play"
```

La commande part au départ de la cue (GO, Goto, chenillard…), repérée « CMD » dans la liste.
Elle accepte aussi toute commande Folie (`Macro 2`, `Go Executor 5`…), séparées par « ; ».
Reaper comprend notamment `/play`, `/stop`, `/marker N`, `/region N`, `/action N`.

**Reaper pilote Folie** : dans Reaper, Préférences > Control/OSC/web > Add > OSC, fichier de
motifs *Default*, envoi (Device IP / port) vers cet ordinateur et le port OSC de Folie. Dans
Folie, cocher « Suivre les marqueurs de Reaper » : le marqueur N fait `Goto Cue N` sur
l'executor sélectionné.

Sur un même ordinateur, Reaper écoute déjà le port 8000 : mettre l'écoute OSC de Folie sur
un autre port (8001 par exemple), sinon l'un des deux ne démarre pas.

## Bibliothèque de fiches

Les fiches livrées sont dans `library/` (format Open Fixture Library). Les vôtres vont
dans la bibliothèque personnelle, à côté des réglages (`library/` du dossier
Folie-DMX). Dans le Patch : **Rechercher en ligne…** trouve un appareil dans
[Open Fixture Library](https://open-fixture-library.org) et importe sa fiche ;
**Importer un fichier…** range une fiche OFL (`.json`) ou GDTF (`.gdtf`, convertie en
OFL : modes, canaux fins, plages, roues, cellules répétées). Une fiche
personnelle de même nom qu'une fiche livrée la remplace : c'est ainsi qu'on la corrige.

**Patch → Modifier la fiche… / Nouvelle fiche…** ouvrent l'éditeur de fiches (aussi dans
le menu Fenêtres) : canaux (nom, 8/16 bits, valeur par défaut, plages typées : couleur
d'émetteur, couleur fixe, shutter, angles de pan/tilt, roue…), modes (ordre DMX, cases
vides, duplication) et cellules (canaux modèles « $pixelKey » répétés par cellule). La
fiche est vérifiée en direct et enregistrée dans la bibliothèque personnelle.
Les shows embarquent leurs fiches et restent lisibles sur une autre machine.

## MIDI

Fenêtre **MIDI** (menu Fenêtres) : choisir les entrées (toutes par défaut) et un port de
retour. **Nouvelle association**, puis toucher un bouton ou un fader du contrôleur :
il est associé, et on choisit l'action (fader, touche, Go, Back, Off ou Flash d'un
executor de la page affichée, Go/Back de l'executor sélectionné, page, blackout,
groupe, ligne de commande). **Profil** remplit les associations d'un contrôleur connu
(Arturia MiniLab 3 : faders et encodeurs sur les faders des executors 1 à 10, encodeur
principal sur les pages, pads sur
leurs touches et leur Flash, pads allumés en couleur, touches blanches du clavier sur les
executors à boutons 11 à 15 et Go/Back). Le retour allume les LED des boutons quand l'executor joue
et renvoie la position des faders (faders motorisés). Le **MIDI Show Control** est
compris (GO avec ou sans numéro de cue, STOP, RESUME, ALL_OFF ; la liste de cues
désigne l'executor) et le **timecode MTC** reçu fait partir les cues : clic droit sur une
cue → Timecode (h:mm:ss.cc, ou « Maintenant » pour le temps reçu). Quand la bande
saute, la séquence se cale sur la dernière cue passée. Un séquenceur pilote Folie
par un port MIDI virtuel (gestionnaire IAC sur macOS, loopMIDI sous Windows).

## Réaction au son

Fenêtre **Son** (menu Fenêtres) : choisir l'entrée (micro, carte son, BlackHole pour la sortie
de Reaper…), le canal, puis **Écouter** (macOS demande l'accès au micro la première fois).
Réactions par executor, enregistrées dans le show : *Pas sur le beat* (Go tous les N beats),
*Fader au niveau* (une bande pilote le fader, entre deux bornes), *Flash sur le beat*.
« Le tempo suit le beat » cale effets et chenillards sur la musique.

## Tests

Tests : `cargo test`. Pour enregistrer une image de l'interface pendant le test
d'interface : `FOLIE_UI_CAPTURE=<dossier> cargo test -p folie-app`.

Voir [ROADMAP.md](ROADMAP.md) pour la suite.

## Licence

Folie-DMX est un logiciel libre, sous licence [GNU GPL v3](LICENSE) ou ultérieure.
Les polices embarquées (Inter, JetBrains Mono) sont sous licence SIL OFL ; les fiches de
projecteurs de `library/` viennent en partie d'Open Fixture Library (MIT, voir `library/README.md`).
