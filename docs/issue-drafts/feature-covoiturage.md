# [BACK] Covoiturage par evenement

## Objectif
Ajouter un module de covoiturage par evenement (voitures, places, inscriptions).

## Contexte
Organisation des trajets pour les participants (pratique pour les BDE).

## Perimetre
- Inclus : model carpool, creation/modification/suppression, inscription et desinscription.
- Exclu : chat, paiement, navigation.

## API / Endpoints
- Routes impactees (methodes, paths) :
  - POST /events/{id}/carpools
  - GET /events/{id}/carpools
  - PATCH /carpools/{id}
  - DELETE /carpools/{id}
  - POST /carpools/{id}/join
  - DELETE /carpools/{id}/join
- Payloads (inputs/outputs) : seats_total, seats_taken, driver_id, origin, depart_at, notes.
- Versioning / compatibilite : nouveaux endpoints.

## Donnees / Migrations
- Tables/collections impactees : carpools, carpool_passengers.
- Migrations necessaires (oui/non) : oui.
- Donnees retro-compatibles (oui/non) : oui (nouvelles tables).

## Regles metier / Validations
- Regles principales : seats_taken <= seats_total; un user ne peut pas s'inscrire deux fois; driver ne peut pas prendre plus de places.
- Cas limites : suppression d'une voiture avec passagers, annulation par le driver.

## Securite / Permissions
- Roles / droits : driver peut editer/supprimer; participants peuvent rejoindre/quitter.
- Donnees sensibles : contact optionnel a proteger (si ajoute plus tard).

## Observabilite
- Logs / metrics / alerting : log creation/suppression; compteur d'inscriptions.

## Tests
- Unitaires : validation des places.
- Integration : join/leave.
- E2E (si applicable) : parcours complet covoiturage.

## Definition of Done
- [ ] API conforme aux specs
- [ ] Migrations appliquees et documentees
- [ ] Tests passes
- [ ] Docs mises a jour (si besoin)

## Notes / Risques
Besoin d'un mecanisme anti-race pour les places restantes.
