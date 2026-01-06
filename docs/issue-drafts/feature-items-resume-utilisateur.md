# [BACK] Resume des items utilisateur par evenement

## Objectif
Exposer un resume des items auxquels un utilisateur participe pour un evenement.

## Contexte
Aujourd'hui il faut parcourir la liste complete des items pour retrouver ce que l'utilisateur doit apporter.

## Perimetre
- Inclus : endpoint "mes items" par evenement, tri et format lisible.
- Exclu : notifications, exports.

## API / Endpoints
- Routes impactees (methodes, paths) : GET /events/{id}/my-items (auth) ou champ my_items dans GET /events/{id}.
- Payloads (inputs/outputs) : liste d'items avec quantite, statut, categorie (si dispo).
- Versioning / compatibilite : ajout d'un endpoint ou champ optionnel.

## Donnees / Migrations
- Tables/collections impactees : items + table de participation.
- Migrations necessaires (oui/non) : non.
- Donnees retro-compatibles (oui/non) : oui.

## Regles metier / Validations
- Regles principales : renvoyer uniquement les items associes a l'utilisateur courant.
- Cas limites : utilisateur non participant, liste vide.

## Securite / Permissions
- Roles / droits : utilisateur connecte, membre de l'evenement.
- Donnees sensibles : aucune.

## Observabilite
- Logs / metrics / alerting : none ou log de debug si besoin.

## Tests
- Unitaires : selection des items par user.
- Integration : endpoint renvoie les bons items.
- E2E (si applicable) : affichage resume cote front.

## Definition of Done
- [ ] API conforme aux specs
- [ ] Tests passes
- [ ] Docs mises a jour (si besoin)

## Notes / Risques
Verifier la performance si la liste d'items est grande.
