# Guide de contribution

Suprnova est open source sous licence MIT, et la contribution la plus
utile est un **bon rapport**. Le projet n'accepte pas les pull
requests : le framework est écrit de bout en bout par les mainteneurs,
et chaque changement passe par eux afin que toute la surface garde une
seule forme. C'est une posture délibérée et permanente - pas une phase
d'avant-1.0.

MIT signifie que vous n'avez jamais besoin de permission pour
poursuivre le code de votre côté : **forkez librement**. Un fork qui
grandit dans sa propre direction est un résultat sain, pas une
rivalité.

Ce que cela signifie en pratique :

- **Rapports de bugs** - bienvenus, via les
  [issues GitHub](https://github.com/entrepeneur4lyf/suprnova/issues).
- **Demandes de fonctionnalités** - bienvenues, via les issues.
  Décrivez le cas d'usage, pas l'implémentation ; une forme prévue
  existe souvent déjà (généralement l'équivalent Laravel).
- **Bugs de documentation** - bienvenus, via les issues. Si un
  chapitre affirme qu'une API existe et que vous ne la trouvez pas,
  c'est un bug de documentation - précisez le chapitre et ce que
  vous attendiez.
- **Problèmes de sécurité** - en privé, par e-mail (voir ci-dessous).
  Jamais en issue publique.
- **Pull requests** - non acceptées. Les PR sont fermées avec un
  renvoi vers ce chapitre ; ouvrez plutôt une issue pour que le
  correctif puisse atterrir en amont, ou forkez et portez le
  changement vous-même.

## Déposer un rapport de bug qui se corrige vite

L'étalon-or est une reproduction depuis un scaffold neuf :

```bash
suprnova new repro-app --frontend vue --no-interaction
# …plus petit changement qui montre le bug…
```

Incluez :

1. **Ce que vous avez fait** - les commandes et le code, réduits au
   minimum
2. **Ce que vous attendiez** - une phrase
3. **Ce qui s'est passé à la place** - la sortie ou l'erreur réelle,
   collée telle quelle
4. **Versions** - le tag du framework (`suprnova --version`, ou le
   `tag =` dans votre `Cargo.toml`) et votre version de Rust
   (`rustc --version`)

Un test qui échoue vaut encore mieux que de la prose. Si vous pouvez
exprimer le bug comme un test contre le framework, collez-le dans
l'issue - il deviendra généralement le test de non-régression avec
lequel le correctif atterrit.

## Compiler depuis les sources (pour investiguer un rapport)

Ce n'est pas nécessaire pour *déposer* une issue, mais reproduire
contre le workspace affine souvent un rapport :

```bash
git clone https://github.com/entrepeneur4lyf/suprnova.git
cd suprnova
cargo check --workspace          # vérifie les types de tout
cargo test --workspace           # exécute la suite complète (~3400 tests)
```

Disposition du workspace : `framework/` (la crate `suprnova`),
`suprnova-cli/` (le binaire `suprnova`), `suprnova-macros/` (macros
proc), `app/` (application de dogfooding interne), `crates/`
(adaptateurs paiements et web-push), et `manual/` (ce manuel).

## Le niveau d'exigence du code

Pas des règles de contribution - mais connaître le standard vous aide
à calibrer vos rapports (une panique venant du code de la
bibliothèque, un test de mode d'échec manquant, ou une API qui force
`unwrap()` mérite toujours un rapport) :

- **Implémentations complètes uniquement.** Pas de TODO, pas de
  scaffolds partiels. Un correctif atterrit avec le test de
  non-régression qui le fixe.
- **Le code de surface publique renvoie `Result`, ne panique pas.**
  Là où un nom infaillible à la Laravel est livré, un homologue
  `try_*` est livré avec lui.
- **Pas d'`unsafe` en dehors de l'amorçage de l'environnement.** Le
  framework a exactement deux blocs `unsafe` dans du code hors tests,
  tous deux dans `config/env.rs::load_dotenv`, tous deux enveloppant
  `std::env::set_var` / `remove_var` - devenus `unsafe` dans
  l'édition 2024 - et tous deux portant une note SAFETY pour
  l'invariant mono-thread au démarrage dont ils dépendent. Tout le
  reste est réservé aux tests. Un nouvel `unsafe` ailleurs nécessite
  une justification écrite en review, et un `unsafe` dans un driver,
  un handler, ou une expansion de macro ne sera pas accepté.
- **`cargo fmt` et clippy sous `-D warnings` font foi.**

Consultez [Modèle d'erreur](error-model.md) pour le contrat d'erreur
complet.

## Sécurité

Signalez les problèmes de sécurité en privé à **shawn@eas4ai.com** (le
mainteneur du projet). Nous accuserons réception sous quelques jours,
travaillerons le correctif sur une branche privée, et coordonnerons la
divulgation avec vous.

Ne déposez pas de problèmes de sécurité en issue GitHub publique tant
qu'un correctif n'a pas été livré.

### Avis de dépendances

`cargo audit` s'exécute dans le gate de release
(`scripts/gate.sh --full`). Si un avis n'a pas de correctif disponible
et que le code vulnérable n'est pas atteignable dans un build par
défaut, il peut être ajouté à `.cargo/audit.toml` - mais chaque
entrée a besoin de trois choses, et `scripts/check-audit.sh` fait
échouer le gate sans elles :

```toml
# OWNER: name <email>
# EXPIRES: YYYY-MM-DD
"RUSTSEC-XXXX-XXXX",
```

- un **propriétaire**, pour que l'exception appartienne à quelqu'un ;
- une **expiration**, après laquelle le gate refuse de s'exécuter tant
  que l'entrée n'est pas renouvelée avec une raison indiquée, ou
  supprimée ;
- un **argument d'atteignabilité écrit** - quel chemin l'importe, et
  pourquoi un build par défaut ne le lie pas.

Les affirmations d'atteignabilité sont vérifiées, pas prises sur
parole. Si votre argument est « c'est derrière une feature désactivée
par défaut », ajoutez l'assertion correspondante à
`scripts/check-feature-matrix.sh`, qui résout les arbres de
dépendances réels et vérifie que la crate est absente de celui par
défaut et présente dans celui activé volontairement. Une exception
dont la justification n'est vérifiée par rien cesse silencieusement
d'être vraie dès que quelqu'un ajoute une dépendance.

Une exception ignorée est une décision de livrer un problème connu.
Elle doit se lire comme telle.

## Licence

MIT, avec attribution au projet amont
[Kit](https://github.com/dayemsiddiqui/kit) que nous avons forké.
