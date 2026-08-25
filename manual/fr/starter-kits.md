# Kits de démarrage

Les kits de démarrage sont des applications Suprnova prêtes à l'emploi que vous
forkez et lancez. Chacun câble les contrôleurs, les routes, les migrations, les
pages frontend et les tests pour une surface produit complète - de sorte que vous
commencez à partir d'une application fonctionnelle, pas d'un scaffold vide.

Deux kits sont disponibles aujourd'hui, modélisés selon la lignée de Laravel.
Choisissez celui qui se rapproche le plus de ce que vous construisez et
personnalisez-le à partir de là.

## Nebula - authentification (niveau Breeze)

**Dépôt : [github.com/eas4ai/Nebula](https://github.com/eas4ai/Nebula)**

Le kit minimal d'authentification complète - l'équivalent Suprnova de Breeze.
Tout ce dont vous avez besoin pour les comptes et rien de plus :

- Inscription avec vérification e-mail
- Connexion avec se souvenir de moi
- Réinitialisation de mot de passe avec réponses anti-énumération
- Gestion du profil - mise à jour e-mail et mot de passe, suppression de compte
- Un frontend Inertia 3 + Svelte 5 de marque (sombre par défaut), avec le
  menu utilisateur connecté câblé

Nebula propose deux suites de test : la logique d'authentification au niveau de
la façade, et une suite HTTP au niveau du câblage qui exécute les vraies routes,
les sessions, les allers-retours CSRF et les portes guest / auth / verified sur
une socket loopback.

Optez pour Nebula quand vous voulez une base de gestion de compte propre pour
construire votre propre produit dessus.

## Pulsar - site produit et communauté

**Dépôt : [github.com/eas4ai/Pulsar](https://github.com/eas4ai/Pulsar)**

Un site complet d'outil développeur / entreprise SaaS sur Vue 3.5 + Vuetify.
Tout ce qui se trouve dans l'histoire d'authentification de Nebula, plus les
surfaces dont un vrai site produit a besoin :

- Page d'accueil marketing et tableau de bord utilisateur
- Un pipeline de documentation Markdown (`docs:build`) avec recherche et table
  des matières générée
- Un système de blog / articles avec un flux RSS
- Profils publics des membres
- Taxonomie - sujets, balises et catégories
- Contrôle d'accès basé sur les rôles : rôles, permissions et portes
- Surfaces d'administration et de modération pour le contenu et les membres

Pulsar est le kit source pour les produits en aval tels que `suprnova.app`.
Optez pour lui quand vous lancez un site produit avec documentation, blog et
communauté de membres - pas juste l'authentification.

## Quel kit ?

| Vous voulez… | Commencer par |
|---|---|
| Des comptes et un endroit pour construire | **Nebula** |
| Un site produit complet - accueil, docs, blog, communauté, RBAC | **Pulsar** |
| Un backend API uniquement (authentification par jeton, pas de frontend) | `suprnova new my-api --api` |

Les deux kits suivent le framework comme une dépendance git et s'exécutent sur
la même pile que vous connaissez déjà - consultez le README de chaque dépôt pour
la configuration. D'autres kits sont prévus ; suivez les
[versions](https://github.com/eas4ai/suprnova/releases) ou ouvrez une
issue si vous en voulez un.

## Ce que le scaffold par défaut vous donne

Si aucun kit ne vous convient, `suprnova new my-app --frontend svelte` (ou
`react`, ou `vue`) livre déjà un flux d'authentification fonctionnel - connexion,
inscription, déconnexion, authentification de session avec le middleware
`authenticate`, protection CSRF et une route `/dashboard` protégée - sur l'un
quelconque des trois frontends (Svelte 5, React 19, Vue 3.5) avec Tailwind v4
et Inertia v3. Consultez [Installation](installation.md) pour la sortie du
scaffold et [Démarrage rapide](quickstart.md) pour la présentation des cinq
premières minutes.

Pour les services API uniquement, `suprnova new my-api --api` initialise Magnetar,
installe le middleware bearer-session, et crée un scaffold pour l'enregistrement et la
connexion par mot de passe contre la table canonique `app_users` sans frontend.

## Contribuer un kit de démarrage

Vous avez construit quelque chose de réutilisable sur Suprnova et vous voulez
le soumettre comme un kit canonique ? Consultez
[Guide de contribution](contributions.md). Nous serions heureux de prendre une
implémentation réelle et la transformer en kit générique.
