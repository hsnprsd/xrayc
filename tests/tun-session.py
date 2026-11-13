"""Exercise real TUN routing inside an isolated, unprivileged Linux namespace.

No routes, DNS settings or interfaces in the user's network namespace are changed.
The upstream SOCKS server is a local fixture; no internet/server credentials needed.
"""
import ipaddress
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import threading
import time

configs, binaries = map(Path, sys.argv[1:])
assert os.getuid() == 0, "Run only inside an unshare user/network namespace"
parent_namespaces = json.loads(os.environ["XRAYC_TEST_NAMESPACES"])
for namespace in ["net", "mnt", "user"]:
    assert os.readlink(f"/proc/self/ns/{namespace}") != parent_namespaces[namespace], f"An isolated {namespace} namespace is required"
# Hide host D-Bus/systemd sockets before testing native DNS setup. Mount changes
# are private to this namespace and cannot reach the user's desktop session.
subprocess.run(["mount", "-t", "tmpfs", "-o", "mode=755", "tmpfs", "/run"], check=True)
subprocess.run(["ip", "link", "set", "lo", "up"], check=True)
subprocess.run(["ip", "link", "add", "physical0", "type", "dummy"], check=True)
subprocess.run(["ip", "addr", "add", "192.0.2.2/24", "dev", "physical0"], check=True)
subprocess.run(["ip", "-6", "addr", "add", "2001:db8:1::2/64", "dev", "physical0", "nodad"], check=True)
subprocess.run(["ip", "link", "set", "physical0", "up"], check=True)
subprocess.run(["ip", "route", "add", "default", "via", "192.0.2.1", "dev", "physical0"], check=True)
subprocess.run(["ip", "-6", "route", "add", "default", "dev", "physical0"], check=True)

seen = []

def exact(stream, size):
    result = b""
    while len(result) < size:
        part = stream.recv(size - len(result))
        if not part:
            raise EOFError()
        result += part
    return result


def address(stream, kind):
    if kind == 1:
        return socket.inet_ntop(socket.AF_INET, exact(stream, 4))
    if kind == 4:
        return socket.inet_ntop(socket.AF_INET6, exact(stream, 16))
    if kind == 3:
        return exact(stream, exact(stream, 1)[0]).decode()
    raise ValueError(kind)


def dns_reply(query):
    return query[:2] + b"\x81\x80" + query[4:6] + b"\x00\x01\x00\x00\x00\x00" + query[12:] + b"\xc0\x0c\x00\x01\x00\x01\x00\x00\x00\x3c\x00\x04\xc0\x00\x02\x2a"


def serve_client(client):
    try:
        with client:
            client.settimeout(20)
            assert exact(client, 1) == b"\x05"
            exact(client, exact(client, 1)[0])
            client.sendall(b"\x05\x00")
            version, command, _, kind = exact(client, 4)
            assert version == 5
            host = address(client, kind)
            port = struct.unpack("!H", exact(client, 2))[0]
            seen.append((host, port))
            if command == 1:
                client.sendall(b"\x05\x00\x00\x01\x7f\x00\x00\x01\x00\x00")
                if port == 53:
                    query = exact(client, struct.unpack("!H", exact(client, 2))[0])
                    reply = dns_reply(query)
                    client.sendall(struct.pack("!H", len(reply)) + reply)
                else:
                    while True:
                        data = client.recv(65536)
                        if not data:
                            break
                        client.sendall(data)
            elif command == 3:
                with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp:
                    udp.bind(("127.0.0.1", 0))
                    udp.settimeout(15)
                    client.sendall(b"\x05\x00\x00\x01\x7f\x00\x00\x01" + struct.pack("!H", udp.getsockname()[1]))
                    while True:
                        packet, sender = udp.recvfrom(65536)
                        kind = packet[3]
                        address_size = 4 if kind == 1 else 16 if kind == 4 else 1 + packet[4]
                        offset = 4 + address_size
                        port = struct.unpack("!H", packet[offset:offset + 2])[0]
                        host = socket.inet_ntop(socket.AF_INET if kind == 1 else socket.AF_INET6, packet[4:offset]) if kind in [1, 4] else packet[5:offset].decode()
                        seen.append((host, port))
                        if port == 53:
                            packet = packet[:offset + 2] + dns_reply(packet[offset + 2:])
                        udp.sendto(packet, sender)
            else:
                raise AssertionError(command)
    except (OSError, EOFError):
        pass


proxy = socket.socket()
proxy.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
proxy.bind(("127.0.0.1", 19080))
proxy.listen()

def accept_clients():
    while True:
        client, _ = proxy.accept()
        threading.Thread(target=serve_client, args=(client,), daemon=True).start()

threading.Thread(target=accept_clients, daemon=True).start()


def run_session(crash=False, cancel=False, service_crash=False, startup_error=False, heartbeat_timeout=False):
    controller = socket.socket()
    controller.bind(("127.0.0.1", 0))
    controller.listen()
    controller.settimeout(15)
    descriptor = configs / "session.json"
    descriptor.write_text(json.dumps({"port": controller.getsockname()[1], "token": "a" * 64}))
    descriptor.chmod(0o600)
    process = subprocess.Popen([str(binaries / "xrayc-service"), "--session", str(descriptor)], stdout=subprocess.PIPE, stderr=subprocess.PIPE, env={**os.environ, "TMPDIR": str(configs)})
    channel = None
    try:
        channel, _ = controller.accept()
        channel.settimeout(30)
        messages = channel.makefile("rwb", buffering=0)
        hello_line = messages.readline()
        assert hello_line, process.stderr.read().decode()
        hello = json.loads(hello_line)
        assert hello["type"] == "hello" and hello["token"] == "a" * 64, hello
        xray = json.loads((configs / "xray.json").read_text())
        if startup_error:
            xray["inbounds"].append({"protocol": "socks", "listen": "127.0.0.1", "port": 19080})
        messages.write((json.dumps({"type": "start", "config": xray}) + "\n").encode())
        if cancel:
            messages.write(b'{"type":"stop"}\n')
        startup = []
        while True:
            event = json.loads(messages.readline())
            startup.append(event)
            if event["type"] == "error":
                assert startup_error and "address already in use" in event["message"], startup
                break
            if event["type"] == ("stopped" if cancel else "ready"):
                assert not startup_error, "A conflicting listener must fail startup"
                break
        if not cancel and not startup_error:
            assert "xrayc0" in subprocess.check_output(["ip", "link", "show"], text=True)
            maps = Path(f"/proc/{process.pid}/maps").read_text()
            assert str(binaries / "libxrayc-core.so") in maps, "Xray must run inside the network service"
            children = [child for task in Path(f"/proc/{process.pid}/task").iterdir()
                        for child in (task / "children").read_text().split()]
            assert children == [], f"The embedded session must not spawn a core process: {children}"
        if heartbeat_timeout:
            event = json.loads(messages.readline())
            assert event["type"] == "stopped", event
        if not any([crash, cancel, service_crash, startup_error, heartbeat_timeout]):
            for family, target in [(socket.AF_INET, "198.18.0.10"), (socket.AF_INET6, "2001:db8::10")]:
                with socket.socket(family, socket.SOCK_STREAM) as request:
                    request.settimeout(10)
                    request.connect((target, 18080))
                    request.sendall(b"transparent TCP over TUN")
                    assert exact(request, 24) == b"transparent TCP over TUN"
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as request:
                request.settimeout(10)
                request.sendto(b"transparent UDP over TUN", ("198.18.0.10", 18081))
                assert request.recv(4096) == b"transparent UDP over TUN"
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as request:
                request.settimeout(10)
                query = b"\x12\x34\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00\x07example\x03com\x00\x00\x01\x00\x01"
                request.sendto(query, ("9.9.9.9", 53))
                response = request.recv(4096)
                assert response[:2] == b"\x12\x34" and response[-4:] == ipaddress.ip_address("192.0.2.42").packed, response
            assert ("9.9.9.9", 53) in seen, seen
            messages.write(b'{"type":"ping"}\n')
            while json.loads(messages.readline())["type"] != "pong":
                pass
            messages.write(b'{"type":"stop"}\n')
            while True:
                event = json.loads(messages.readline())
                if event["type"] == "stopped":
                    break
        if service_crash:
            process.kill()
        # An abrupt control-channel loss must also close the core and restore routing.
        messages.close()
        channel.close()
        channel = None
        assert process.wait(timeout=15) == (-9 if service_crash else 1 if startup_error else 0), process.stderr.read().decode()
        deadline = time.monotonic() + 10
        while "xrayc0" in subprocess.check_output(["ip", "link", "show"], text=True) and time.monotonic() < deadline:
            time.sleep(0.1)
        assert "xrayc0" not in subprocess.check_output(["ip", "link", "show"], text=True)
        assert "172.28.233" not in subprocess.check_output(["ip", "route", "show", "table", "all"], text=True)
        restored = subprocess.check_output(["ip", "route", "get", "198.18.0.10"], text=True)
        assert "via 192.0.2.1 dev physical0" in restored, restored
    finally:
        if channel is not None:
            channel.close()
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
        controller.close()


run_session()
print("PASS: transparent IPv4 TCP, IPv6 TCP, UDP and DNS; clean disconnect")
run_session(crash=True)
print("PASS: controller crash restores routes and removes the TUN interface")
run_session(cancel=True)
print("PASS: cancellation during startup leaves the network unchanged")
run_session(service_crash=True)
print("PASS: network service crash removes the embedded TUN interface and restores routing")
run_session(startup_error=True)
print("PASS: partial startup failure closes the embedded core and restores routing")
run_session(heartbeat_timeout=True)
print("PASS: heartbeat timeout closes the embedded core and restores routing")

# Exercise the non-elevated connection manager too. This namespace already has
# its own administrator capabilities; replace only the authorization prompt.
authorization = configs / "authorization"
authorization.mkdir()
policy = authorization / "pkexec"
policy.write_text('#!/bin/sh\nexec "$@"\n')
policy.chmod(0o755)
manager = subprocess.Popen([str(binaries / "xrayc-service"), "--controller"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                           env={**os.environ, "TMPDIR": str(configs), "PATH": f"{authorization}:{os.environ['PATH']}"}, text=True, bufsize=1)
try:
    source = (configs / "xray.json").read_text()
    manager.stdin.write(json.dumps({"id": 1, "type": "connect", "text": source}) + "\n")
    manager.stdin.flush()
    phases = []
    while True:
        response = json.loads(manager.stdout.readline())
        if response.get("event") == "status":
            phases.append(response["status"]["phase"])
            assert phases[-1] != "error", response
            if phases[-1] == "connected":
                break
        elif response.get("id") == 1:
            assert response["result"] == {"ok": True}, response
    assert phases[0] == "connecting", phases
    with socket.create_connection(("198.18.0.10", 18080), timeout=10) as request:
        request.sendall(b"managed transparent VPN")
        assert exact(request, 23) == b"managed transparent VPN"
    manager.stdin.write('{"id":2,"type":"disconnect"}\n')
    manager.stdin.flush()
    phases = []
    while True:
        response = json.loads(manager.stdout.readline())
        if response.get("event") == "status":
            phases.append(response["status"]["phase"])
            if phases[-1] == "disconnected":
                break
    assert phases == ["disconnecting", "disconnected"], phases
    manager.stdin.close()
    assert manager.wait(timeout=20) == 0, manager.stderr.read()
    assert "xrayc0" not in subprocess.check_output(["ip", "link", "show"], text=True)
finally:
    if manager.poll() is None:
        manager.kill()
        manager.wait()
print("PASS: connection manager start/stop with simulated administrator authorization")
