"""Keep the newest published stable binary release after verifying its download."""
import hashlib
import json
import os
import re
import urllib.error
import urllib.parse
import urllib.request

REPOSITORY = "MidiAug/xmu-sports"


class GitHub:
    def __init__(self, token):
        self.token = token

    def request(self, path, method="GET"):
        request = urllib.request.Request("https://api.github.com/repos/" + REPOSITORY + "/" + path,
            method=method, headers={"Authorization": "Bearer " + self.token,
                "Accept": "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28"})
        with urllib.request.urlopen(request, timeout=60) as response:
            return json.load(response) if response.status != 204 else None

    def releases(self):
        output = []
        for page in range(1, 101):
            batch = self.request(f"releases?per_page=100&page={page}")
            output.extend(batch)
            if len(batch) < 100:
                return output
        raise RuntimeError("Release listing exceeded page limit")


def download(asset):
    # Public asset download never carries the API administration token across redirects.
    if not asset["browser_download_url"].startswith("https://github.com/" + REPOSITORY + "/releases/download/"):
        raise ValueError("Unexpected public download URL")
    return urllib.request.urlopen(asset["browser_download_url"], timeout=120)


def verify(release):
    tag = release["tag_name"]
    if release["draft"] or release["prerelease"] or not re.fullmatch(r"v\d+\.\d+\.\d+", tag):
        raise ValueError("Only a stable published release can replace old releases")
    name = f"XMU.Sports_{tag[1:]}_x64-setup.exe"
    assets = release["assets"]
    if len(assets) != 2 or {a["name"] for a in assets} != {name, "SHA256SUMS.txt"}:
        raise ValueError("Newest release must contain exactly installer and checksum")
    installer = next(a for a in assets if a["name"] == name)
    checksum = next(a for a in assets if a["name"] == "SHA256SUMS.txt")
    with download(checksum) as response:
        manifest = response.read(4097)
    if len(manifest) > 4096:
        raise ValueError("Checksum manifest too large")
    expected, filename = manifest.decode("utf-8-sig").strip().split("  ")
    if filename != name or not re.fullmatch(r"[0-9a-f]{64}", expected):
        raise ValueError("Checksum manifest mismatch")
    digest, size = hashlib.sha256(), 0
    with download(installer) as response:
        while chunk := response.read(1024 * 1024):
            digest.update(chunk)
            size += len(chunk)
    if size != installer["size"] or digest.hexdigest() != expected:
        raise ValueError("Newest installer download failed integrity verification")
    return expected


def prune(api):
    latest = api.request("releases/latest")
    digest = verify(latest)
    releases = api.releases()
    # Ignore draft work in progress and pre-releases; these are not the stable channel.
    old = [r for r in releases if not r["draft"] and not r["prerelease"] and r["id"] != latest["id"]]
    if api.request("releases/latest")["id"] != latest["id"]:
        raise RuntimeError("Latest release changed during verification; retry on the new version")
    for release in old:
        api.request("releases/" + str(release["id"]), "DELETE")
        tag = release["tag_name"]
        if tag != latest["tag_name"]:
            try:
                api.request("git/refs/tags/" + urllib.parse.quote(tag, safe=""), "DELETE")
            except urllib.error.HTTPError as error:
                if error.code != 404:
                    raise
        print("Removed old public version:", tag)
    print("Kept:", latest["tag_name"], "SHA256:", digest)


if __name__ == "__main__":
    if os.environ.get("GITHUB_REPOSITORY") != REPOSITORY:
        raise SystemExit("Run only in the public distribution repository")
    prune(GitHub(os.environ["GH_TOKEN"]))
