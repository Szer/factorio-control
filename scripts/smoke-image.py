import http.client
import json
import os
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
name = "factorio-control-test-" + uuid.uuid4().hex[:12]
volume = name + "-data"
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


def request(path, body=None, authenticated=True, request_origin=origin):
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=60)
    headers = {"Host": host, "Origin": request_origin}
    if cookie and authenticated:
        headers["Cookie"] = cookie
    if body is not None:
        headers["Content-Type"] = "application/json"
    try:
        connection.request(
            "POST" if body is not None else "GET",
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
    while time.monotonic() < deadline:
        try:
            if request("/api/auth/status", authenticated=False)[0] == 200:
                return
        except (OSError, http.client.HTTPException):
            pass
        time.sleep(0.2)
    raise RuntimeError("Image did not become ready")


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
    mount = volume + ":/data" + (":U" if engine[-1] == "podman" else "")
    command(
        "run", "-d", "--name", name, "--read-only", "--tmpfs", "/tmp:rw,nosuid,nodev,size=256m",
        "--cap-drop", "ALL", "--security-opt", "no-new-privileges", "--pids-limit", "512",
        "-p", "127.0.0.1::8080", "-p", "127.0.0.1::34197/udp",
        "-e", "FC_PUBLIC_URL=" + origin, "--volume", mount, image,
    )
    port = int(command("port", name, "8080/tcp").stdout.strip().splitlines()[0].rsplit(":", 1)[1])
    wait_ready()
    require(request("/api/status", authenticated=False)[0] == 401, "Anonymous access was allowed")
    command("exec", name, "/app/factorio-control", "--healthcheck")
    require(command("exec", name, "id", "-u").stdout.strip() == "10001", "Image is not running as UID 10001")
    require(command("exec", name, "stat", "-c", "%a", "/data/bootstrap-admin.json").stdout.strip() == "600", "Bootstrap permissions are not private")
    credentials = json.loads(command("exec", name, "cat", "/data/bootstrap-admin.json").stdout)
    status, headers, account = request("/api/auth/login", credentials, authenticated=False)
    require(status == 200 and account["user"]["must_change_password"], "Temporary account was not restricted")
    session_cookie = headers.get("set-cookie", headers.get("Set-Cookie", ""))
    require("Secure" in session_cookie and "HttpOnly" in session_cookie and "SameSite=Strict" in session_cookie, "Public cookie protections are missing")
    cookie = session_cookie.split(";", 1)[0]
    require(request("/api/status")[0] == 403, "Temporary account accessed game controls")
    password = secrets.token_urlsafe(32)
    require(request("/api/auth/password", {"current_password": credentials["password"], "password": password})[0] == 204, "Password change failed")
    require(command("exec", name, "test", "!", "-e", "/data/bootstrap-admin.json", check=False).returncode == 0, "Bootstrap file remains after rotation")
    require(request("/api/status")[0] == 401, "Old session survived rotation")
    status, headers, account = request("/api/auth/login", {"username": "admin", "password": password}, authenticated=False)
    require(status == 200 and not account["user"]["must_change_password"], "Changed account could not sign in")
    cookie = headers.get("set-cookie", headers.get("Set-Cookie", "")).split(";", 1)[0]
    require(request("/api/server/stop", {}, request_origin="https://attacker.invalid")[0] == 403, "Cross-origin mutation was allowed")
    status, _, value = request("/api/status")
    require(status == 200 and value["version"] == Path(".factorio-version").read_text().strip(), "Bundled Factorio version differs from configured version")
    require(request("/api/saves/create", {"name": "smoke-world.zip"})[0] == 201, "World creation failed")
    require(request("/api/server/start", {"save": "smoke-world.zip"})[0] == 200, "Game start failed")
    wait_state({"running", "paused"})
    require(request("/api/server/command", {"command": "/players", "acknowledge_scripting": False})[0] == 200, "Game console command failed")
    require(request("/api/server/stop", {})[0] == 200, "Graceful stop failed")
    wait_state({"stopped"})
    require(request("/api/server/start", {"save": "smoke-world.zip"})[0] == 200, "Second game start failed")
    wait_state({"running", "paused"})
    command("restart", "--time", "150", name, timeout=170)
    wait_ready()
    require(request("/api/status")[0] == 401, "Session survived restart")
    require(command("exec", name, "test", "!", "-e", "/data/bootstrap-admin.json", check=False).returncode == 0, "Restart recreated bootstrap credentials")
    require(request("/api/auth/login", {"username": "admin", "password": password}, authenticated=False)[0] == 200, "Account did not persist")
    print("Image passed: non-root, bridge networking, authentication, password rotation, world creation, game lifecycle and restart persistence.")
finally:
    command("rm", "--force", name, check=False)
    command("volume", "rm", volume, check=False)
