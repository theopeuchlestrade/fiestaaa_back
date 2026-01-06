# [BACK] Playlist partagee pour evenement

## Objectif
Ajouter un support de playlist partagee par evenement (Spotify/Apple Music/Deezer) avec validation cote serveur.

## Contexte
Besoin d'un espace musique commun pour les participants.

## Perimetre
- Inclus : stockage du lien, type de service, lecture dans le GET evenement, ecriture par createur.
- Exclu : synchronisation temps reel, import de titres, edition de playlist.

## API / Endpoints
- Routes impactees (methodes, paths) : GET /events/{id}, PATCH /events/{id} (ou PATCH /events/{id}/playlist).
- Payloads (inputs/outputs) : playlist_url (nullable), playlist_provider (spotify|apple_music|deezer|other).
- Versioning / compatibilite : ajout de champs optionnels retro-compatibles.

## Donnees / Migrations
- Tables/collections impactees : events.
- Migrations necessaires (oui/non) : oui, ajout playlist_url + playlist_provider.
- Donnees retro-compatibles (oui/non) : oui (valeurs null par defaut).

## Regles metier / Validations
- Regles principales : seul createur/admin peut modifier; provider connu; url valide; suppression possible.
- Cas limites : provider inconnu, url invalide, url vide.

## Securite / Permissions
- Roles / droits : createur, admin.
- Donnees sensibles : aucune.

## Observabilite
- Logs / metrics / alerting : log d'update playlist avec event_id + provider.

## Tests
- Unitaires : validation provider/url.
- Integration : update event + lecture.
- E2E (si applicable) : creation evenement avec playlist.

## Definition of Done
- [ ] API conforme aux specs
- [ ] Migrations appliquees et documentees
- [ ] Tests passes
- [ ] Docs mises a jour (si besoin)

## Notes / Risques
Validation des URLs par provider (regex ou parsing).
