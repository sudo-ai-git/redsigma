#!/usr/bin/env python3
"""
Test script to verify the REDΣSIGMA GitHub Pages demo wallet connection (NEAR Connect).

Checks that:
- NEAR Connect is loaded via importmap + esm.sh (ES module)
- type="module" script imports NearConnector
- the redirect-based requestSignIn is REMOVED (no full-page wallet redirect)
- the dead helperUrl (HTTP 404) is removed from near-api-js connect()
- the broken WalletConnect path is removed (bridge dead + web3-provider is Ethereum not NEAR)
- near-api-js is still present for read-only view calls (connect only, no sign-in)
- signAndSendTransaction is present for state-changing calls
- docs/ and frontend/ are identical (GitHub Pages serves docs/)
"""

import re
import sys


def check(path, label):
    try:
        with open(path) as f:
            content = f.read()
    except FileNotFoundError:
        print(f"❌ {label}: file not found: {path}")
        return False

    results = {}

    # NEAR Connect importmap + module
    results["importmap present"] = bool(re.search(r'type="importmap"', content))
    results["near-connect esm.sh"] = bool(re.search(r'esm\.sh/@hot-labs/near-connect', content))
    results["NearConnector imported"] = bool(re.search(r'import\s*{\s*NearConnector\s*}\s*from', content))
    results["module script"] = bool(re.search(r'type="module"', content))

    # near-api-js still present (for read-only view calls, connect only)
    results["near-api-js loaded"] = bool(re.search(r'near-api-js@4\.0\.0', content))

    # NEAR Connect API surface
    results["NearConnector instantiated"] = bool(re.search(r'new NearConnector', content))
    results["signAndSendTransaction present"] = bool(re.search(r'signAndSendTransaction', content))
    results["getAccounts present"] = bool(re.search(r'getAccounts', content))

    # NO redirect-based sign-in (the user's core complaint)
    results["no requestSignIn redirect"] = not bool(re.search(r'requestSignIn\s*\(', content))
    results["no window.location.assign"] = not bool(re.search(r'window\.location\.assign', content))
    results["no new WalletConnection"] = not bool(re.search(r'new WalletConnection', content))

    # dead helperUrl must be absent
    results["helperUrl removed"] = not bool(re.search(r'helperUrl', content))

    # broken WalletConnect libs must be absent
    results["no web3-provider"] = not bool(re.search(r'@walletconnect/web3-provider', content))
    results["no web3.js"] = not bool(re.search(r'web3@1\\.10\\.0', content))
    results["no WalletConnectProvider"] = not bool(re.search(r'WalletConnectProvider', content))
    results["no web3 variable"] = not bool(re.search(r'\bweb3\b', content))

    print(f"=== {label} ===")
    all_ok = True
    for name, ok in results.items():
        print(f"  {'✅' if ok else '❌'} {name}: {ok}")
        all_ok = all_ok and ok
    return all_ok


def main():
    frontend = "/home/sudosudo/.openclaw/workspace/redsigma/frontend/index.html"
    docs = "/home/sudosudo/.openclaw/workspace/redsigma/docs/index.html"

    f_ok = check(frontend, "frontend/index.html")
    d_ok = check(docs, "docs/index.html")

    # frontend and docs must be identical (GitHub Pages serves docs/)
    with open(frontend) as f:
        fe = f.read()
    with open(docs) as f:
        de = f.read()
    identical = fe == de
    print(f"\n{'✅' if identical else '❌'} frontend and docs identical: {identical}")

    ok = f_ok and d_ok and identical
    print(f"\n=== Final: {'✅ PASSED' if ok else '❌ FAILED'} ===")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
