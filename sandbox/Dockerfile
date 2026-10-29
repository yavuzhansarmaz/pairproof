FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends openssh-server python3 nodejs ca-certificates mariadb-server mariadb-client iptables \
    && rm -rf /var/lib/apt/lists/* \
    && rm -f /etc/ssh/ssh_host_* \
    && mkdir -p /run/sshd /workspace /var/run/mysqld /run/mysqld \
    && chown mysql:mysql /var/run/mysqld /run/mysqld

COPY sshd_config /etc/ssh/sshd_config
COPY entrypoint.sh /usr/local/bin/pairproof-entrypoint.sh
RUN chmod 0755 /usr/local/bin/pairproof-entrypoint.sh

WORKDIR /workspace
EXPOSE 22
ENTRYPOINT ["/usr/local/bin/pairproof-entrypoint.sh"]
