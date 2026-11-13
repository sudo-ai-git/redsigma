#!/usr/bin/env python3
"""
Test script to verify the REDΣSIGMA GitHub Pages demo wallet connection is fixed.

Checks that:
- near-api-js is loaded (NEAR testnet wallet connection requires it)
- the dead helperUrl (HTTP 404) is removed from connect()
- the broken WalletConnect path is removed (bridge dead + web3-provider is Ethereum not NEAR)
- the undefined walletOption ReferenceError bug is gone
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

    # near-api-js must be loaded
    results["near-api-js loaded"] = bool(re.search(r'near-api-js@4\.0\.0', content))

    # dead helperUrl must be absent
    results["helperUrl removed"] = not bool(re.search(r'helperUrl', content))

    # broken WalletConnect libs must be absent
    results["no web3-provider"] = not bool(re.search(r'@walletconnect/web3-provider', content))
    results["no web3.js"] = not bool(re.search(r'web3@1\.10\.0', content))
    results["no WalletConnectProvider"] = not bool(re.search(r'WalletConnectProvider', content))
    results["no initWalletConnect"] = not bool(re.search(r'initWalletConnect', content))
    results["no web3 variable"] = not bool(re.search(r'\bweb3\b', content))

    # the undefined walletOption bug must be absent
    results["no walletOption bug"] = not bool(re.search(r'\bwalletOption\b', content))

    # NEAR wallet path must be present
    results["WalletConnection present"] = bool(re.search(r'new WalletConnection', content))
    results["requestSignIn present"] = bool(re.search(r'requestSignIn', content))
    results["connect present"] = bool(re.search(r'\bconnect\(', content))

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
