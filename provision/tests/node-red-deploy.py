#!/usr/bin/env python3
"""Adds flows to Node-RED beside the ones it has, through its admin API, for the examples' test
(converge-test.sh, 10c).

    node-red-deploy.py <user> <password> <flow.json>...

Nodes whose id Node-RED has already are left out, so it can run twice.
"""

import json
import sys
import urllib.parse
import urllib.request

BASE = "http://127.0.0.1:1880"


def call(path, body=None, headers=None, form=False):
    data = None
    if body is not None:
        data = urllib.parse.urlencode(body).encode() if form else json.dumps(body).encode()
    request = urllib.request.Request(BASE + path, data=data, headers=headers or {})
    with urllib.request.urlopen(request, timeout=30) as answer:
        return json.load(answer)


def main():
    user, password, files = sys.argv[1], sys.argv[2], sys.argv[3:]
    login = {"client_id": "node-red-admin", "grant_type": "password", "scope": "*", "username": user, "password": password}
    token = call("/auth/token", login, form=True)["access_token"]
    headers = {"Authorization": f"Bearer {token}", "Node-RED-API-Version": "v2"}
    current = call("/flows", headers=headers)
    have = {node["id"] for node in current["flows"]}
    added = [node for name in files for node in json.load(open(name, encoding="utf-8")) if node["id"] not in have]
    headers.update({"Content-Type": "application/json", "Node-RED-Deployment-Type": "full"})
    call("/flows", {"rev": current["rev"], "flows": current["flows"] + added}, headers)
    print(f"Added {len(added)} nodes.")


if __name__ == "__main__":
    main()
