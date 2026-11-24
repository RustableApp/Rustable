"""Disposable RESP stub that refuses handshakes without the expected TLS SNI.

Used only by test-redis-mtls.sh. Requires mutual TLS; never accesses user data.
"""
import pathlib
import socket
import ssl
import sys
import threading

certs = pathlib.Path(sys.argv[1])
context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain(certs / "server.crt", certs / "server.key")
context.load_verify_locations(certs / "ca.crt")
context.verify_mode = ssl.CERT_REQUIRED
context.sni_callback = lambda sock, name, ctx: (
    None if name == "redis.fixture.test" else ssl.ALERT_DESCRIPTION_UNRECOGNIZED_NAME
)


def serve(socket):
    try:
        socket.settimeout(3)
        with context.wrap_socket(socket, server_side=True) as stream:
            with stream.makefile("rb") as source:
                for _ in range(32):
                    header = source.readline(128)
                    if not header:
                        return
                    assert header.startswith(b"*")
                    count = int(header[1:])
                    assert 0 < count <= 16
                    args = []
                    for _ in range(count):
                        header = source.readline(128)
                        assert header.startswith(b"$")
                        size = int(header[1:])
                        assert 0 <= size <= 4096
                        args.append(source.read(size))
                        assert source.read(2) == b"\r\n"
                    stream.sendall(b"+PONG\r\n" if args[0] == b"PING" else b"+OK\r\n")
    except (OSError, ValueError, AssertionError):
        socket.close()


with socket.socket() as listener:
    listener.bind(("127.0.0.1", 0))
    listener.listen(8)
    (certs / "sni-port").write_text(str(listener.getsockname()[1]))
    while True:
        peer, _ = listener.accept()
        threading.Thread(target=serve, args=(peer,), daemon=True).start()
