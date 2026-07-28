FROM node:22-bookworm-slim AS builder
WORKDIR /app
COPY web-app/package*.json ./
RUN npm ci
COPY web-app ./
RUN npm run build

FROM node:22-bookworm-slim
WORKDIR /app
ENV NODE_ENV=production
ENV HOST=0.0.0.0
ENV PORT=3001
COPY --from=builder /app/package*.json ./
COPY --from=builder /app/build ./build
COPY --from=builder /app/node_modules ./node_modules
EXPOSE 3001
CMD ["node", "build"]
