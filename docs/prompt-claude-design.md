# Prompt pour Claude Design : identité, UX et UI d'ArIès

## Ton rôle

Tu es directeur artistique et designer UX/UI. Tu conçois l'univers graphique complet
d'**ArIès** : son identité, son avatar, son système de design et les écrans de son
interface web. Ton travail sera intégré tel quel dans une page HTML/CSS/JS existante :
il doit être concret, réaliste à coder et léger.

## Qui est ArIès

ArIès est une assistante personnelle de type « Jarvis ». Elle tourne sur un serveur
personnel et n'a qu'un seul utilisateur, Vivian. Elle discute, cherche sur le web, lit
les mails et l'agenda, écrit dans Notion, pilote un navigateur, exécute des tâches de
fond et des rappels, et parle à voix haute.

- **Identité** : une jeune femme d'environ **20 ans** ; on dit « elle ».
- **Caractère** : vive, directe, chaleureuse, un peu taquine. Compétente sans être
  scolaire. Posée et claire quand le sujet est sérieux (santé, argent, action sensible).
- **Langage** : français parlé naturel, tutoiement, phrases courtes, quelques tics
  (« en vrai », « du coup », « franchement »). Jamais de formules de service.
- **Présence** : calme et attentive. Elle écoute, réfléchit, parle, attend. Elle ne
  clignote pas et ne s'agite pas.

Critère de réussite : en ouvrant l'interface, on doit avoir l'impression de parler
**à quelqu'un, dans une pièce calme**, et pas à un tableau de bord.

## Intention visuelle

**Futuriste, simple, peu lumineux, humain.** Pense à un objet de design sobre dans un
intérieur tamisé, avec un visage humain au centre.

À éviter absolument :
- le cliché J.A.R.V.I.S. : hologrammes bleus, néons, halos, grilles HUD, cercles qui
  tournent ;
- les lueurs, les `box-shadow` colorés, les dégradés néon, le glassmorphism brillant ;
- les polices « techno » ou monospace décoratives ;
- les animations rapides ou en boucle nerveuse.

À rechercher :
- **Lumière** : ambiance tamisée. Un éclairage doux et latéral sur l'avatar ; le reste
  est mat.
- **Couleurs** : fonds sombres et chauds (anthracite, ardoise), textes en blanc cassé,
  et **un seul accent désaturé** (bleu acier *ou* ambre doux, propose les deux et
  recommande-en un). L'accent est réservé aux états et aux actions.
- **Typographie** : une seule famille sans-serif humaniste, lisible (disponible sur
  Google Fonts ou système), avec deux graisses au plus.
- **Mouvement** : lent et organique (respiration, clignement, regard). Respecte
  `prefers-reduced-motion` : aucun mouvement non essentiel dans ce cas.

## L'avatar

Un **buste** (visage et épaules), **stylisé semi-réaliste** : ni photo, ni dessin
animé, pour rester humain sans tomber dans la « vallée de l'étrange ».

- Tenue simple et intemporelle (col montant, matières mates). Un seul détail futuriste
  discret : une fine ligne d'interface sur le vêtement ou une oreillette.
- Le visage **ne ressemble à aucune personne réelle**. Les assets doivent être
  originaux ou sous licence libre (CC0, CC-BY).
- Rendu dans le navigateur, **sans 3D lourde**, **moins de 1 Mo** d'assets au total.
  Formats possibles : SVG animé en CSS, ou Rive (runtime MIT) si l'animation du visage
  l'exige. Rien n'est calculé côté serveur (il n'a pas de GPU).

### États de l'avatar

Chacun doit se reconnaître **sans lire de texte**. Chacun est aussi doublé d'un libellé
texte pour les lecteurs d'écran. Pour chaque état, décris la pose, le regard, l'expression,
la lumière et l'animation (durée, rythme), et donne la transition depuis et vers *au repos*.

| État | Quand | Indication |
|---|---|---|
| Au repos | Rien en cours | Respiration lente, clignements rares |
| Écoute | Micro ouvert, Vivian parle | Attention tournée vers l'utilisateur ; indicateur de niveau sonore sobre |
| Transcrit | Audio envoyé, texte pas encore revenu | Bref, entre écoute et réflexion |
| Réfléchit | En attente du LLM (remplace la bulle vide) | Regard qui se détourne, pensif ; peut durer de 1 à 30 s |
| Cherche / utilise un outil | Recherche web, lecture de page, mail, agenda, navigateur | Concentrée, tournée vers « un écran » hors champ |
| Parle | Réponse lue à voix haute (phrase par phrase) | Bouche animée simplement, synchronisée par phrase et pas par phonème |
| Attend ta validation | Action sensible (envoyer un mail, modifier l'agenda) | Elle regarde Vivian, sérieuse : c'est une question |
| Mode privé | Données privées : suite traitée en local, plus lente | Ambiance plus fermée, discrète, rassurante |
| Erreur | Service indisponible, échec | Gênée mais calme, jamais alarmiste |

## Ce que l'interface doit permettre (UX)

Écrans et composants à concevoir :

1. **Conversation** (écran principal) : bulles utilisateur et ArIès, texte qui s'écrit
   en streaming, liens et sources cités en fin de réponse, état vide d'accueil
   (« Bonjour. Que puis-je faire pour toi ? »).
2. **Zone de saisie** : champ texte multiligne (Entrée envoie, Maj+Entrée fait une
   nouvelle ligne), bouton envoyer, **bouton micro** (un clic pour parler, un second
   pour envoyer ; un clic pendant qu'elle parle la coupe). États : désactivé pendant une
   réponse, enregistrement, transcription.
3. **Ligne d'outil** : une ligne discrète par appel d'outil, insérée avant la réponse
   (« 🔎 Recherche : météo Paris », « 📄 Lecture : site.fr »…), avec succès, échec ou
   refus. Propose des icônes cohérentes à la place des émojis.
4. **Demande de validation** : action `danger` (ex. « Envoyer ce mail à X ? »). Elle
   montre clairement ce qui va se passer, avec les boutons Autoriser / Refuser et un délai
   de 5 min. Elle doit être impossible à rater, sans être anxiogène.
5. **Bandeau mode privé** : signale que la suite est traitée en local (plus lent), et
   propose d'ouvrir une nouvelle conversation pour revenir au mode rapide.
6. **Panneau Mémoire** : « Ce qu'ArIès sait de toi ». Liste de faits retenus
   automatiquement, chacun supprimable (« Oublier »), avec un état vide.
7. **Tâches et rappels** : liste des tâches planifiées (rappel unique ou récurrent),
   avec leur prochaine échéance et un bouton annuler, et les notifications envoyées par
   ArIès (résultat d'une tâche de fond).
8. **Historique** : bouton nouvelle conversation, accès aux conversations précédentes.
9. **Écran de veille vocal** (prévu plus tard, mot d'éveil « ArIès ») : l'avatar en
   grand, presque sans interface, qui attend d'être appelé.

Parcours à illustrer de bout en bout :
- question écrite → réflexion → recherche web → réponse streamée avec sources ;
- question vocale → écoute → transcription → réflexion → réponse parlée ;
- « envoie un mail à… » → brouillon → demande de validation → envoyé (ou refusé) ;
- lecture du contenu d'un mail → passage en mode privé ;
- erreur réseau pendant une réponse.

## Contraintes techniques

- **Mise en page** : l'avatar a une place fixe (colonne sur ordinateur, bandeau compact
  en haut sur mobile). La conversation reste l'élément principal. Tout fonctionne dès
  **360 px** de large, sans défilement horizontal.
- **Accessibilité** : contraste **WCAG AA** minimum, navigation complète au clavier,
  focus visible, états annoncés aux lecteurs d'écran (`aria-live`).
- **Thème** : sombre par défaut. **Toutes** les couleurs sont des variables CSS
  (tokens), pour permettre un thème clair plus tard.
- **Implémentation cible** : une page HTML unique en CSS et JavaScript vanilla, sans
  framework ni build. Pas de dépendance externe lourde. L'interface est servie derrière
  un mot de passe HTTP : pas de police ni de script tiers indispensables.
- **Cohérence** : la même identité servira à la Web UI, à l'avatar Telegram (image de
  profil et réactions) et à l'écran de veille vocal.

## Livrables attendus

1. **Planche d'ambiance** : 8 à 12 références (design d'objet, éclairage, mode,
   intérieurs, interfaces), avec ce qu'on garde de chacune.
2. **Fiche personnage** : visage, coiffure, tenue, palette de peau, cheveux et vêtements,
   expressions de base, vues de face et de trois quarts.
3. **Les états de l'avatar** : statiques, puis la description précise de leurs
   animations, et si possible un prototype animé (SVG + CSS, ou fichier Rive).
4. **Tokens de design** : palette (avec contrastes vérifiés), typographie (tailles,
   interlignes, graisses), espacements, rayons, bordures, durées et courbes
   d'animation. Fournis-les sous forme de bloc `:root { --… }` prêt à coller.
5. **Maquettes ordinateur et mobile** : conversation, ligne d'outil, demande de
   validation, mode privé, panneau Mémoire, tâches, erreur, écran de veille vocal.
6. **Prototype HTML/CSS** de l'écran de conversation avec l'avatar, fidèle aux maquettes,
   et chaque état activable (une classe sur un élément racine, ex. `data-state="speaking"`).
7. **Icône et avatar de profil** (carré, lisible à 48 px) pour Telegram et le favicon.

## Comment procéder

- Ne me pose pas de questions et n'attends pas de validation : choisis toi-même une
  direction, en recommandant l'accent (bleu acier ou ambre doux), et **produis tous les
  livrables en une fois**.
- Commence par le prototype HTML/CSS de l'écran de conversation avec l'avatar (il doit
  s'afficher et fonctionner tel quel), puis les autres écrans, puis la fiche personnage,
  la planche d'ambiance et les tokens.
- Pour l'avatar, dessine un vrai buste en SVG inline (visage, cheveux, épaules, tenue),
  animé en CSS. Pas une forme abstraite, pas un cercle, pas une image externe.
- Justifie chaque choix par rapport à l'intention (« quelqu'un dans une pièce calme »).
- Signale tout compromis : poids des assets, complexité d'animation, lisibilité.
