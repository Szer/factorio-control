import http.client
import json
import os
import re
from pathlib import Path
import secrets
import shlex
import subprocess
import sys
import time
import uuid


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


engine = shlex.split(os.environ.get("CONTAINER_ENGINE", "docker"))
image = sys.argv[1]
direct_http = "--direct-http" in sys.argv[2:]
name = "factorio-control-test-" + uuid.uuid4().hex[:12]
volume = name + "-data"
network = name + "-network"
proxy = name + "-proxy"
origin = "https://control.example.invalid"
host = "control.example.invalid"
port = None
cookie = None


def command(*arguments, check=True, timeout=30):
    result = subprocess.run(
        [*engine, *arguments], capture_output=True, text=True, timeout=timeout
    )
    if check and result.returncode:
        raise RuntimeError("Container command failed: " + arguments[0])
    return result


def request(path, body=None, authenticated=True, request_origin=None, method=None, client_ip="198.51.100.50"):
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=60)
    headers = {"Host": host, "Origin": request_origin or origin, "X-Forwarded-For": client_ip}
    if cookie and authenticated:
        headers["Cookie"] = cookie
    if body is not None:
        headers["Content-Type"] = "application/json"
    try:
        connection.request(
            method or ("POST" if body is not None else "GET"),
            path,
            body=None if body is None else json.dumps(body),
            headers=headers,
        )
        response = connection.getresponse()
        content = response.read(1024 * 1024)
        return response.status, dict(response.getheaders()), json.loads(content) if content else None
    finally:
        connection.close()


def wait_ready():
    deadline = time.monotonic() + 45
    last_status = None
    while time.monotonic() < deadline:
        try:
            last_status = request("/api/auth/status", authenticated=False)[0]
            if last_status == 200:
                return
        except (OSError, http.client.HTTPException):
            pass
        time.sleep(0.2)
    state = command("inspect", "--format", "{{json .State}}", name, check=False).stdout
    logs = command("logs", "--tail", "20", name, check=False)
    diagnostic = state + logs.stdout + logs.stderr
    print("Readiness HTTP status:", last_status, file=sys.stderr)
    print(re.sub(r"[A-Za-z0-9_+/=-]{32,}", "[redacted]", diagnostic), file=sys.stderr)
    raise RuntimeError("Image did not become ready")


def connect_container():
    global port, host, origin
    port = int(command("port", name if direct_http else proxy, "8080/tcp" if direct_http else "8081/tcp").stdout.strip().splitlines()[0].rsplit(":", 1)[1])
    if direct_http:
        host = f"127.0.0.1:{port}"
        origin = f"http://{host}"
    wait_ready()


def wait_state(states):
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline:
        status, _, value = request("/api/status")
        require(status == 200, "Authenticated game status failed")
        require(value["state"] != "failed", "Factorio failed to start")
        if value["state"] in states:
            return
        time.sleep(0.2)
    raise RuntimeError("Game lifecycle operation timed out")


try:
    command("volume", "create", volume)
    command("network", "create", network)
    if not direct_http:
        proxy_code = r"""
import http.client
import http.server

class Proxy(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def forward(self):
        body = self.rfile.read(int(self.headers.get('Content-Length', '0')))
        upstream = http.client.HTTPConnection('control', 8080, timeout=60)
        try:
            upstream.request(self.command, self.path, body, dict(self.headers))
            response = upstream.getresponse()
            content = response.read()
            self.send_response(response.status)
            for key, value in response.getheaders():
                if key.lower() not in ('content-length', 'transfer-encoding', 'connection'):
                    self.send_header(key, value)
            self.send_header('Content-Length', str(len(content)))
            self.end_headers()
            self.wfile.write(content)
        finally:
            upstream.close()

    do_GET = forward
    do_POST = forward
    do_PUT = forward

http.server.ThreadingHTTPServer(('0.0.0.0', 8081), Proxy).serve_forever()
    """
        command("run", "-d", "--name", proxy, "--network", network,
                "--read-only", "--tmpfs", "/tmp", "--cap-drop", "ALL",
                "--security-opt", "no-new-privileges", "--pids-limit", "64",
                "-p", "127.0.0.1::8081", "docker.io/library/python:3.12-alpine", "python", "-c", proxy_code, timeout=120)
        proxy_ip = json.loads(command("inspect", proxy).stdout)[0]["NetworkSettings"]["Networks"][network]["IPAddress"]
        require(proxy_ip, "Isolated reverse proxy has no transport address")
    mount = volume + ":/data" + (":U" if engine[-1] == "podman" else "")
    environment = ["-e", "FC_SESSION_TTL_SECONDS=3600"] if direct_http else ["-e", "FC_PUBLIC_URL=" + origin, "-e", "FC_TRUSTED_PROXIES=" + proxy_ip]
    container_arguments = (
        "run", "-d", "--name", name, "--network", network, "--network-alias", "control", "--read-only", "--tmpfs", "/tmp:rw,nosuid,nodev,size=256m",
        "--cap-drop", "ALL", "--security-opt", "no-new-privileges", "--pids-limit", "512",
        "-p", "127.0.0.1::8080", "-p", "127.0.0.1::34197/udp",
        *environment, "--volume", mount, image,
    )
    command(*container_arguments)
    connect_container()
    require(request("/api/status", authenticated=False)[0] == 401, "Anonymous access was allowed")
    command("exec", name, "/app/factorio-control", "--healthcheck")
    require(command("exec", name, "id", "-u").stdout.strip() == "10001", "Image is not running as UID 10001")
    require(command("exec", name, "stat", "-c", "%a", "/data/bootstrap-admin.json").stdout.strip() == "600", "Bootstrap permissions are not private")
    credentials = json.loads(command("exec", name, "cat", "/data/bootstrap-admin.json").stdout)
    status, headers, account = request("/api/auth/login", credentials, authenticated=False)
    require(status == 200 and account["user"]["must_change_password"], "Temporary account was not restricted")
    session_cookie = headers.get("set-cookie", headers.get("Set-Cookie", ""))
    lifetime = 3600 if direct_http else 2592000
    require(f"Max-Age={lifetime}" in session_cookie, "Session cookie lifetime differs from configuration")
    require(abs(account["expires_at"] - time.time() - lifetime) < 10, "Server session lifetime differs from configuration")
    require(("Secure" in session_cookie) == (not direct_http) and "HttpOnly" in session_cookie and "SameSite=Strict" in session_cookie, "Session cookie protections do not match the configured scheme")
    cookie = session_cookie.split(";", 1)[0]
    require(request("/api/status")[0] == 403, "Temporary account accessed game controls")
    password = secrets.token_urlsafe(32)
    require(request("/api/auth/password", {"current_password": credentials["password"], "password": password})[0] == 204, "Password change failed")
    require(command("exec", name, "test", "!", "-e", "/data/bootstrap-admin.json", check=False).returncode == 0, "Bootstrap file remains after rotation")
    require(request("/api/status")[0] == 401, "Old session survived rotation")
    status, headers, account = request("/api/auth/login", {"username": "admin", "password": password}, authenticated=False)
    require(status == 200 and not account["user"]["must_change_password"], "Changed account could not sign in")
    cookie = headers.get("set-cookie", headers.get("Set-Cookie", "")).split(";", 1)[0]
    if not direct_http:
        for attempt in range(6):
            status = request("/api/auth/login", {"username": "admin", "password": "incorrect-fixture"}, authenticated=False, client_ip="198.51.100.99")[0]
            require(status == (429 if attempt == 5 else 401), f"Proxy client throttling is not isolated: attempt {attempt + 1}, status {status}")
        require(request("/api/auth/login", {"username": "admin", "password": password}, authenticated=False)[0] == 200, "An attack from another proxy client blocked valid login")
    require(request("/api/server/stop", {}, request_origin="https://attacker.invalid")[0] == 403, "Cross-origin mutation was allowed")
    status, _, value = request("/api/status")
    require(status == 200 and value["version"] == Path(".factorio-version").read_text().strip(), "Bundled Factorio version differs from configured version")
    require(request("/api/saves/create", {"name": "smoke-world.zip"})[0] == 201, "World creation failed")
    command("exec", name, "sh", "-c", "printf '[\"fixture-player\"]\\n' > /data/server-whitelist.json")
    command("exec", name, "sh", "-c", "printf '%s\\n' '[{\"username\":\"banned-fixture\",\"reason\":\"isolated review test\"}]' > /data/server-banlist.json")
    require(request("/api/server/start", {"save": "smoke-world.zip"})[0] == 409, "Migrated whitelist silently defaulted to disabled")
    status, _, manager = request("/api/manager-settings")
    require(status == 200, "Manager settings unavailable")
    manager["settings"]["use_server_whitelist"] = True
    require(request("/api/manager-settings", manager, method="PUT")[0] == 200, "Whitelist could not be enabled")
    require(request("/api/server/start", {"save": "smoke-world.zip"})[0] == 200, "Game start failed")
    wait_state({"running", "paused"})
    require(request("/api/server/command", {"command": "Factorio 1.1", "acknowledge_scripting": False})[0] == 200, "Version-spoof chat fixture failed")
    time.sleep(0.2)
    require(request("/api/status")[2]["version"] == Path(".factorio-version").read_text().strip(), "Chat changed the trusted engine version")
    require(request("/api/server/command", {"command": "/players", "acknowledge_scripting": False})[0] == 200, "Game console command failed")
    require(request("/api/server/stop", {})[0] == 200, "Graceful stop failed")
    wait_state({"stopped"})
    require(request("/api/server/start", {"save": "smoke-world.zip"})[0] == 200, "Second game start failed")
    wait_state({"running", "paused"})
    session_before_restart = request("/api/auth/status")[2]
    command("restart", "--time", "150", name, timeout=170)
    connect_container()
    require(request("/api/status")[0] == 200, "Session did not survive restart")
    require(request("/api/auth/status")[2] == session_before_restart, "Restart changed session identity or expiry")
    command("rm", "--force", name)
    command(*container_arguments)
    connect_container()
    require(request("/api/status")[0] == 200, "Session did not survive container recreation")
    require(request("/api/auth/status")[2] == session_before_restart, "Container recreation changed session identity or expiry")
    require(request("/api/auth/logout", {})[0] == 204, "Logout failed")
    command("restart", "--time", "150", name, timeout=170)
    connect_container()
    require(request("/api/status")[0] == 401, "Logout was undone by restart")
    require(command("exec", name, "sh", "-c", 'test "$(stat -c %a /data/panel-sessions.json)" = 600', check=False).returncode == 0, "Session file is not private")
    require(command("exec", name, "test", "!", "-e", "/data/bootstrap-admin.json", check=False).returncode == 0, "Restart recreated bootstrap credentials")
    require(request("/api/auth/login", {"username": "admin", "password": password}, authenticated=False)[0] == 200, "Account did not persist")
    if direct_http:
        # The successful post-restart login consumes one of the five attempts.
        for attempt in range(5):
            status = request("/api/auth/login", {"username": "admin", "password": "incorrect-fixture"}, authenticated=False, client_ip=f"198.51.100.{attempt + 1}")[0]
            require(status == (429 if attempt == 4 else 401), "Untrusted forwarding headers bypassed direct HTTP login throttling")
    print("Image passed (" + ("direct HTTP" if direct_http else "HTTPS proxy") + "): non-root, bridge networking, authentication, password rotation, world creation, game lifecycle and restart persistence.")
finally:
    command("rm", "--force", name, check=False)
    command("rm", "--force", proxy, check=False)
    command("volume", "rm", volume, check=False)
    command("network", "rm", network, check=False)
