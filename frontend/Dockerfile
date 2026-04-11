# Stage 1: Build Angular
FROM node:20-alpine AS builder

WORKDIR /app

COPY package*.json ./
RUN npm ci

COPY . .

ARG API_URL=http://localhost:8080
ENV API_URL=$API_URL
RUN sh set-env.sh && npx ng build --configuration production

# Stage 2: Serve with nginx
FROM nginx:1.27-alpine

RUN rm /etc/nginx/conf.d/default.conf

# Copy nginx template (uses $PORT)
COPY nginx.conf.template /etc/nginx/templates/default.conf.template

# Copy built Angular app
COPY --from=builder /app/dist/apexstore-frontend/browser /usr/share/nginx/html

# nginx official image automatically runs envsubst on /etc/nginx/templates/*.template
# and outputs to /etc/nginx/conf.d/ before starting nginx
ENV PORT=80

EXPOSE 80

CMD ["nginx", "-g", "daemon off;"]
