# Commandes de Folie-DMX

Folie reprend un sous-ensemble de la syntaxe de la grandMA2. Une commande se tape :

- dans la ligne de commande `Folie>` en bas de l'écran, puis Entrée ;
- dans l'onglet Commande de la télécommande web ;
- dans une macro (une commande par ligne) ;
- dans le champ « Au départ » d'une cue (plusieurs commandes séparées par `;`) ;
- en OSC, à l'adresse `/folie/cmd` (voir le README).

Les mots-clés ne tiennent pas compte des majuscules et s'abrègent tant qu'il n'y a pas
d'ambiguïté : `fi 1 t 4 at 50` vaut `Fixture 1 Thru 4 At 50`. Dans la ligne de commande,
Tab complète le mot en cours, les flèches haut et bas rappellent les commandes passées,
et `?` (ou F1) ouvre cette aide.

## Sélectionner

| Commande | Effet |
|---|---|
| `Fixture 1` | sélectionne la fixture 1 |
| `Fixture 1 Thru 4` | les fixtures 1 à 4 |
| `Fixture 1 + 3 - 2` | ajoute la 3, retire la 2 |
| `Fixture 5.3` | la cellule 3 de la fixture 5 (projecteur à plusieurs cellules) |
| `Group 2` | le contenu du groupe 2 |
| `Group 2 + Fixture 7` | groupes et fixtures se mélangent |

Une sélection tapée complète la précédente, sauf après une valeur : `Fixture 1 At 50`
puis `Fixture 2` repart de zéro.

## Donner des valeurs

| Commande | Effet |
|---|---|
| `Fixture 1 Thru 4 At 50` | dimmer à 50 % |
| `At 70` | sur la sélection en cours |
| `Group 2 At Full` | dimmer à 100 % |
| `At + 10` | 10 points de plus que la valeur en cours |
| `At - 10` | 10 points de moins |
| `Fixture 1 Attribute "Pan" At 50` | un autre paramètre que le dimmer, par son nom |
| `Preset 4.1` | applique le preset 4.1 à la sélection |
| `Effect 2` | lance l'effet modèle 2 sur la sélection |
| `Fixture 2 Release` | marque la sélection comme relâchée : la cue enregistrée la relâchera |
| `Release` | idem, sur la sélection en cours |

Les presets s'écrivent famille.numéro. Familles :

| Famille | Numéro |
|---|---|
| Dimmer | 1 |
| Position | 2 |
| Gobo | 3 |
| Couleur | 4 |
| Faisceau | 5 |
| Focus | 6 |
| Contrôle | 7 |

Effets modèles : 1 Sinus dimmer, 2 Chenillard, 3 Vague, 4 Cercle, 5 Arc-en-ciel,
6 Pulse, 7 Scintillement. Ceux du pool « Mes effets » se lancent depuis la fenêtre Effets.

## Programmer

| Commande | Effet |
|---|---|
| `Clear` | vide la sélection ; une seconde fois, vide le programmer |
| `ClearAll` | vide la sélection et le programmer d'un coup |
| `Blind` | entre en blind (ou en sort) : on prépare sans toucher la scène |
| `Blind Off` | sort du blind |

## Enregistrer, renommer, supprimer

| Commande | Effet |
|---|---|
| `Store Group 3 "Cour"` | la sélection devient le groupe 3 (remplacé s'il existe) |
| `Store Preset 4.1 "Rouge"` | les valeurs de couleur du programmer deviennent le preset 4.1 |
| `Store Cue 3` | le programmer devient la cue 3 de l'executor sélectionné |
| `Store Cue` | à la suite de la dernière cue |
| `Store Cue 3 Fade 4 "Entrée"` | avec un fondu de 4 s et un nom |
| `Store Executor 3` | le programmer devient la cue suivante de l'executor 3 |
| `Store Macro 3 "Ballet"` | crée la macro 3 (ses lignes s'écrivent dans la fenêtre Macros) |
| `Label Group 3 "Jardin"` | renomme ; de même `Label Preset 4.1`, `Label Cue 3`, `Label Macro 3` |
| `Delete Group 3` | supprime ; de même `Delete Preset 4.1`, `Delete Cue 3`, `Delete Macro 3` |

Une cue qui existe déjà est complétée, pas remplacée. Un executor sans séquence en reçoit
une nouvelle. `Fade` ne vaut que pour les cues.

## Jouer les executors et les cues

L'executor sélectionné est celui du GO principal : `Go`, `Back`, `Off`, `Goto Cue`,
`Store Cue`, `Assert` sans numéro s'appliquent à lui, comme la touche Espace. On le voit :

- en haut de l'écran, à côté de GO et Back : « Exec 1 », en jaune ;
- dans la fenêtre Executors, son numéro est en jaune ;
- dans les fenêtres Cues et Tracking, dans le champ EXECUTOR ;
- sur la télécommande, sous GO et Back, et par un liseré jaune sur sa carte.

On le choisit :

- en cliquant sur le numéro d'un executor dans la fenêtre Executors ;
- avec le champ EXECUTOR des fenêtres Cues et Tracking (une place de la page en cours) ;
- en tapant `Executor 3` (ou `Executor 2.5` pour la page 2, place 5) ;
- avec `Store Executor 3`, qui enregistre dans l'executor 3 et le sélectionne.

`Go Executor 2` joue l'executor 2 sans changer la sélection.

| Commande | Effet |
|---|---|
| `Go` | cue suivante de l'executor sélectionné |
| `Back` | cue précédente |
| `Off` | éteint l'executor sélectionné |
| `Go Executor 2` | sur l'executor 2 de la page en cours ; de même `Back Executor 2`, `Off Executor 2` |
| `Go Executor 2.5` | page 2, place 5 |
| `Off All` | éteint tous les executors, sauf ceux qui sont protégés |
| `Goto Cue 5` | va à la cue 5 de l'executor sélectionné |
| `Executor 3` | sélectionne l'executor 3 (il devient celui du GO principal) |
| `Executor 3 At 50` | son fader à 50 % |
| `Assert` | l'executor sélectionné reprend la main sur ce qu'il tient |
| `Assert Executor 3` | idem pour l'executor 3 |
| `Fix Executor 3` | fixe l'executor 3 sur toutes les pages, ou le défixe |
| `Block Cue 3` | écrit en dur dans la cue 3 les valeurs qu'elle suit |
| `Unblock Cue 3` | retire de la cue 3 les valeurs identiques à celles qu'elle suivrait |
| `Page 2` | affiche la page 2 d'executors |
| `Page +` | page suivante |
| `Page -` | page précédente |
| `Tempo 128` | tempo des chenillards et des effets, de 1 à 999 BPM |

Chaque page a 20 places : 1 à 10 avec fader, 11 à 20 en boutons. Il y a 99 pages.
Sans page, un executor fixé l'emporte sur celui de la page en cours.

## Macros

| Commande | Effet |
|---|---|
| `Macro 2` | lance la macro 2 |
| `Wait 2` | dans une macro seulement : attend 2 s avant la ligne suivante |

## Sons

| Commande | Effet |
|---|---|
| `Sound 3` | lance le son 3 (et ceux qui le suivent) |
| `Sound 3 Off` | l'arrête net |
| `Sound 3 Off Fade 2` | l'arrête en fondu de 2 s |
| `Sound Off` | arrête tous les sons |
| `Sound Off Fade 2` | tous, en fondu de 2 s |

La pré-écoute de la fenêtre Sons ne passe pas par ces commandes : `Sound Off` ne coupe pas
le casque.

## OSC

| Commande | Effet |
|---|---|
| `Osc "Reaper" "/play"` | message OSC vers la destination « Reaper » du show |
| `Osc "Reaper" "/marker" 3` | avec une valeur : entier, réel (`0.5`), négatif ou texte entre guillemets |

Les destinations se règlent dans le menu Télécommande.

## Abréviations

| Mot | S'écrit aussi |
|---|---|
| Fixture | `fi` |
| Group | `g` |
| Preset | `p` |
| Thru | `t` |
| Back | `b` |
| Off | `o` |
| Executor | `exec` |
| Effect | `eff` |

Les autres mots s'abrègent par leur début, tant qu'un seul mot commence ainsi :
`stor`, `del`, `lab`, `mac`. Un début ambigu est refusé avec la liste des possibilités :
`c` peut être Clear, ClearAll ou Cue.

## Raccourcis clavier

Hors de la ligne de commande :

| Touche | Effet |
|---|---|
| Espace | Go de l'executor sélectionné |
| S | arme Store : le clic suivant sur un groupe, un preset, une cue ou un executor y enregistre (Échap désarme) |
| B | Blackout |
| R | le programmer lâche la sélection |
| Maj-R | Release : la sélection est marquée relâchée, pour la prochaine cue |
| F1 | cette aide |
| Cmd 1, Cmd 2… | ouvre ou ferme une fenêtre (voir le menu Fenêtres) |
| Option Cmd 1 à 9 | rappelle une disposition des fenêtres |
| Cmd S, Cmd O, Cmd N | enregistrer, ouvrir, nouveau show |

Sous Windows, Ctrl remplace Cmd et Alt remplace Option.

## Ce qui manque par rapport à la grandMA2

Pas encore de Copy, Move, Update, Assign, Oops ni Highlight. Une seule valeur par
commande (`At 50`, pas de `At 0 Thru 100`). Les cues se rangent sur la séquence de
l'executor sélectionné, sans `Sequence 3` en ligne de commande.
