# [BACK] Separation items apporte vs besoin

## Objectif
Distinguer les items "Ce que j'apporte" (tout le monde) et "Ce que j'ai besoin" (createur).

## Contexte
Le modele actuel melange les items demandes par le createur et les propositions des participants.

## Perimetre
- Inclus : ajout d'un type d'item (bring/need) et regles d'acces associees.
- Exclu : categories d'items (soft/alcool, etc) traitees dans une autre issue.

## API / Endpoints
- Routes impactees (methodes, paths) : endpoints items (create/update/list) pour accepter item_kind.
- Payloads (inputs/outputs) : item_kind enum (need|bring).
- Versioning / compatibilite : champs optionnel avec valeur par defaut.

## Donnees / Migrations
- Tables/collections impactees : items.
- Migrations necessaires (oui/non) : oui, ajout item_kind.
- Donnees retro-compatibles (oui/non) : oui, valeur par defaut "need" pour items existants.

## Regles metier / Validations
- Regles principales : createur/admin peut creer/modifier "need"; participants peuvent creer "bring".
- Cas limites : conversion d'un item "need" en "bring" et inversement; suppression.

## Securite / Permissions
- Roles / droits : createur/admin vs participant.
- Donnees sensibles : aucune.

## Observabilite
- Logs / metrics / alerting : log lors de changement de type d'item.

## Tests
- Unitaires : validation de item_kind et permissions.
- Integration : creation d'items need vs bring.
- E2E (si applicable) : separation visible cote front.

## Definition of Done
- [ ] API conforme aux specs
- [ ] Migrations appliquees et documentees
- [ ] Tests passes
- [ ] Docs mises a jour (si besoin)

## Notes / Risques
Impact possible sur l'UI et le tri existant.
