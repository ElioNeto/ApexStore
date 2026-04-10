#!/bin/sh
# Substitui o placeholder %%API_URL%% pelo valor da variável de ambiente API_URL
# Usado durante o build na Railway

API_URL=${API_URL:-http://localhost:8080}

echo "Setting API_URL to: $API_URL"

sed -i "s|%%API_URL%%|$API_URL|g" src/environments/environment.prod.ts

echo "environment.prod.ts updated:"
cat src/environments/environment.prod.ts
