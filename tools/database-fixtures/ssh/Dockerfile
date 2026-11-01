FROM python:3.12-alpine
RUN apk add --no-cache openssh-server socat && passwd -d root
COPY sshd_config /etc/ssh/sshd_config
COPY start.sh /start.sh
CMD ["sh", "/start.sh"]
