# Docker image (headless daemon)

A teljes útmutató: [docs/install-docker.md](../../docs/install-docker.md).

```bash
docker build -f packaging/docker/Dockerfile -t ferrent .
docker run -d --name ferrent -e FERRENT_TOKEN=change-me \
    -p 50129:50129 -v ferrent-data:/var/lib/ferrent ferrent
```
