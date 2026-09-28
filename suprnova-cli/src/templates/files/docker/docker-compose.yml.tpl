services:
  # PostgreSQL Database
  postgres:
    image: postgres:16-alpine
    container_name: {project_name}_postgres
    restart: unless-stopped
    environment:
      POSTGRES_USER: ${DB_USER:-suprnova}
      POSTGRES_PASSWORD: ${DB_PASSWORD:-{db_password}}
      POSTGRES_DB: ${DB_NAME:-suprnova_db}
    # Published on the loopback interface only. A bare "5432:5432" binds
    # 0.0.0.0 on the Docker host, which puts a development database on
    # every interface the machine has - including the public one, on a
    # laptop on a café network or any cloud VM without a firewall.
    # Override DB_HOST_BIND if you genuinely need to reach it from
    # another host.
    ports:
      - "${DB_HOST_BIND:-127.0.0.1}:${DB_PORT:-5432}:5432"
    volumes:
      - postgres_data:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U ${DB_USER:-suprnova} -d ${DB_NAME:-suprnova_db}"]
      interval: 10s
      timeout: 5s
      retries: 5

  # Redis Cache
  redis:
    image: redis:7-alpine
    container_name: {project_name}_redis
    restart: unless-stopped
    # Loopback only - see the note on the postgres service. Redis ships
    # with no authentication at all, so an exposed port is an open shell
    # onto the cache (and, via CONFIG SET, onto the filesystem).
    ports:
      - "${REDIS_HOST_BIND:-127.0.0.1}:${REDIS_PORT:-6379}:6379"
    volumes:
      - redis_data:/data
    healthcheck:
      test: ["CMD", "redis-cli", "ping"]
      interval: 10s
      timeout: 5s
      retries: 5
{mailpit_service}{minio_service}
volumes:
  postgres_data:
  redis_data:{additional_volumes}

networks:
  default:
    name: {project_name}_network
