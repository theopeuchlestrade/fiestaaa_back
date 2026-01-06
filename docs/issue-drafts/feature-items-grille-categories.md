# [BACK] Grille items + categories + resume quantites

## Objectif
Ajouter des categories simples aux items et fournir un resume des quantites par categorie.

## Contexte
La liste d'items est longue et difficile a parcourir. On veut filtrer et voir les quantites rapidement.

## Perimetre
- Inclus : champ category sur item, validation des valeurs, resume par categorie.
- Exclu : rendu UI (gere cote front).

## API / Endpoints
- Routes impactees (methodes, paths) : endpoints items pour accepter category; GET /events/{id}/items (ou summary) renvoie la categorie.
- Payloads (inputs/outputs) : category enum (soft|alcool|sale|sucre|autre), quantite si dispo.
- Versioning / compatibilite : champs optionnel retro-compatible.

## Donnees / Migrations
- Tables/collections impactees : items.
- Migrations necessaires (oui/non) : oui, ajout item_category.
- Donnees retro-compatibles (oui/non) : oui, valeur par defaut "autre".

## Regles metier / Validations
- Regles principales : categorie obligatoire pour nouvel item; valeurs fermees.
- Cas limites : items existants sans categorie, conversion.

## Securite / Permissions
- Roles / droits : selon regles items actuelles.
- Donnees sensibles : aucune.

## Observabilite
- Logs / metrics / alerting : none.

## Tests
- Unitaires : validation category.
- Integration : creation item + lecture resume.
- E2E (si applicable) : affichage grille + filtre cote front.

## Definition of Done
- [ ] API conforme aux specs
- [ ] Migrations appliquees et documentees
- [ ] Tests passes
- [ ] Docs mises a jour (si besoin)

## Notes / Risques
Clarifier la source de "quantite" (nombre d'inscrits vs champ explicite).
