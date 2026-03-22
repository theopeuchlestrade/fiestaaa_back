# Politique de sécurité

## Versions prises en charge

Les correctifs de sécurité ciblent en priorité :

- la branche `main` ;
- la version actuellement déployée en production ;
- les secrets et l'infrastructure associés au déploiement documenté dans `docs/deploiement.md`.

Les branches anciennes, forks non maintenus ou déploiements dérivés ne sont pas garantis.

## Signaler une vulnérabilité

Ne créez pas d'issue publique pour signaler une faille de sécurité.

Canal recommandé quand le dépôt sera public :

- GitHub Private Vulnerability Reporting, une fois activé.

Tant que ce mécanisme n'est pas disponible :

- signalez la vulnérabilité au mainteneur via un canal privé déjà établi ;
- demandez explicitement un canal d'échange sécurisé si vous devez transmettre un secret, un PoC sensible ou des journaux contenant des données privées ;
- évitez toute divulgation publique avant validation du correctif.

## Ce qu'il faut inclure dans le signalement

Merci d'inclure, si possible :

- le composant concerné ;
- l'impact attendu ;
- les prérequis d'exploitation ;
- des étapes de reproduction ;
- un PoC minimal si vous en avez un ;
- les versions ou commits concernés.

## Attentes de divulgation

Objectif côté maintenance :

- accuser réception rapidement ;
- confirmer si le problème est bien une vulnérabilité ;
- préparer un correctif ou une mitigation ;
- coordonner la divulgation une fois le risque réduit.

Avant le passage public et l'activation de GitHub Private Vulnerability Reporting, cette politique doit être lue avec `docs/incident-securite.md` et `docs/passage-public-open-source.md`.
