# VPS provisioning

Ce dossier rend le VPS reproductible sans stocker de secrets dans Git.

- `cloud-init.yml` : bootstrap d'un VPS neuf au moment de sa creation.
- `ansible/playbook.yml` : remise en conformite idempotente d'un VPS existant.
- `ansible/inventory.example.yml` : inventaire a copier localement en `inventory.yml`.

La stack applicative reste decrite par `../../docker-compose.prod.yml`. Les secrets de production restent dans GitHub Actions `production` et sont materialises sur le VPS par les workflows de deploy.

## Option 1: VPS neuf avec cloud-init

1. Copier `cloud-init.yml`.
2. Remplacer les placeholders, surtout la cle publique SSH.
3. Coller le contenu dans le champ cloud-init/user-data du fournisseur VPS.
4. Attendre la fin du bootstrap, puis verifier:

```bash
ssh deploy@51.75.20.71
docker compose version
cd ~/apps/fiestaaa
ls -la
```

Le `cloud-init` prepare le port SSH de production `1969`. Gardez toujours la session initiale ouverte tant qu'une nouvelle connexion sur `1969` n'a pas ete validee.

## Option 2: VPS existant avec Ansible

Installer Ansible localement:

```bash
brew install ansible
```

Copier puis adapter l'inventaire:

```bash
cp infra/vps/ansible/inventory.example.yml infra/vps/ansible/inventory.yml
```

Appliquer:

```bash
ansible-playbook -i infra/vps/ansible/inventory.yml infra/vps/ansible/playbook.yml
```

Faire un dry-run partiel quand le serveur est deja configure:

```bash
ansible-playbook -i infra/vps/ansible/inventory.yml infra/vps/ansible/playbook.yml --check
```

`inventory.yml` est ignore par Git et peut contenir l'IP, l'utilisateur SSH initial ou des chemins locaux. Les secrets applicatifs ne doivent pas y etre stockes.

## Option 3: provisionnement depuis GitHub Actions

Le workflow manuel `Provision VPS` execute le meme playbook depuis GitHub Actions, avec les secrets de l'environnement `production`.

Secrets requis:

- `VPS_HOST`
- `VPS_SSH_KEY`
- `VPS_USER` si vous ne renseignez pas `connection_user`
- `VPS_PORT` optionnel, sinon `22` pour un VPS fraichement reinstalle ; la cible de production est `1969`

Pour un VPS neuf, lancez le workflow avec `connection_user=root` si l'acces root par cle SSH est actif. Pour un VPS deja configure, laissez `connection_user` vide pour reutiliser `VPS_USER`.

Le workflow derive la cle publique depuis `VPS_SSH_KEY` et l'ajoute au compte `deploy`.

## Premier deploiement complet

Les workflows normaux mettent a jour un seul service a la fois et attendent que les deux tags d'images existent deja dans `~/apps/fiestaaa/.env`. Pour initialiser une machine vierge:

1. Lancer `Provision VPS`.
2. Dans `fiestaaa_front`, lancer `Build and Deploy Front` avec `skip_deploy=true`. Noter le SHA du commit front utilise comme tag d'image.
3. Dans `fiestaaa_back`, lancer `Bootstrap VPS Stack` avec `front_image_tag=<sha_front>`. Ce workflow build/push l'API puis lance toute la stack Compose.
4. Ensuite, utiliser les workflows normaux de deploy back/front.

Si vous utilisez FCM HTTP v1, stockez aussi la cle de service Firebase en base64 dans `FCM_SERVICE_ACCOUNT_JSON_B64` sur l'environnement GitHub `production` du backend:

```bash
base64 < service-account.json | tr -d '\n' | gh secret set FCM_SERVICE_ACCOUNT_JSON_B64 --repo theopeuchlestrade/fiestaaa_back --env production
```

## Ce que le playbook gere

- paquets systeme de base ;
- installation Docker Engine + Compose V2 depuis le depot Docker officiel ;
- utilisateur `deploy` ;
- cles SSH autorisees pour `deploy` ;
- UFW pour SSH, HTTP et HTTPS ;
- Fail2ban pour SSH ;
- arborescence `~/apps/fiestaaa` ;
- copie de `docker-compose.prod.yml` vers `~/apps/fiestaaa/docker-compose.yml` ;
- permissions de `traefik/letsencrypt/acme.json`, `.env` et `data/service-account.json`.

## Ce qui reste volontairement hors Git

- valeurs de `.env` ;
- `service-account.json` Firebase ;
- cle privee SSH de deploy ;
- token GHCR ;
- keystores Android ;
- backups de base de donnees.

Pour un VPS completement neuf, l'ordre pratique est:

1. cloud-init ou Ansible ;
2. DNS `fiestaaa.app` et `api.fiestaaa.app` vers l'IP publique ;
3. secrets GitHub `production` remplis ;
4. premier deploiement back/front depuis GitHub Actions ;
5. verification `docker compose ps` et smoke checks publics.
