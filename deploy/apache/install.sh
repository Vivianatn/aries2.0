#!/usr/bin/env bash
# Ajoute ArIès à l'Apache existant sans toucher aux autres sites. À lancer en root :
#   bash deploy/apache/install.sh <utilisateur>
# Le mot de passe est demandé de façon interactive, ou lu dans $ARIES_PASSWORD.
set -euo pipefail

ARIES_USER="${1:?usage : install.sh <utilisateur>}"
DOMAIN="aries.antoine-vivian.fr"
HERE="$(cd "$(dirname "$0")" && pwd)"

echo "==> Sauvegarde de /etc/apache2"
tar czf "/root/apache2-backup-$(date +%Y%m%d-%H%M%S).tgz" -C /etc apache2

echo "==> Mot de passe d'accès"
if [[ -n "${ARIES_PASSWORD:-}" ]]; then
  printf '%s' "$ARIES_PASSWORD" | htpasswd -i -B -c /etc/apache2/aries.htpasswd "$ARIES_USER"
else
  htpasswd -B -c /etc/apache2/aries.htpasswd "$ARIES_USER"
fi
chown root:www-data /etc/apache2/aries.htpasswd
chmod 640 /etc/apache2/aries.htpasswd

echo "==> Virtual host"
install -m 644 "$HERE/aries.conf" /etc/apache2/sites-available/aries.conf
a2ensite aries.conf
if ! apachectl configtest; then
  a2dissite aries.conf
  echo "Configuration invalide : site désactivé, Apache n'a pas été rechargé." >&2
  exit 1
fi
systemctl reload apache2

echo "==> Certificat HTTPS (certificat séparé, celui des autres sites n'est pas modifié)"
certbot --apache --non-interactive --agree-tos --register-unsafely-without-email \
  --cert-name "$DOMAIN" -d "$DOMAIN" --redirect

echo "ArIès est derrière https://$DOMAIN"
