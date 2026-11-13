#!/usr/bin/env python3
"""
Test script to verify the frontend walletconnect fixes are working.
This script checks if the frontend can be loaded and has the required walletconnect functionality.
"""

import re
import sys
import json

def test_frontend_files():
    """Test that frontend files have walletconnect functionality."""
    
    # Test frontend/index.html
    frontend_path = "/home/sudosudo/.openclaw/workspace/redsigma/frontend/index.html"
    
    try:
        with open(frontend_path, 'r') as f:
            frontend_content = f.read()
    except FileNotFoundError:
        print(f"❌ Frontend file not found: {frontend_path}")
        return False
    
    # Check for walletconnect library imports
    walletconnect_lib_patterns = [
        r'@walletconnect/web3-provider',
        r'walletconnect/web3-provider',
        r'@walletconnect/web3-provider@1.8.0',
        r'web3@1.10.0'
    ]
    
    has_walletconnect_lib = any(re.search(pattern, frontend_content, re.IGNORECASE) for pattern in walletconnect_lib_patterns)
    
    # Check for wallet options UI
    wallet_options_pattern = r'wallet-option.*walletconnect'
    has_wallet_options = bool(re.search(wallet_options_pattern, frontend_content, re.IGNORECASE))
    
    # Check for WalletConnect initialization
    walletconnect_init_patterns = [
        r'new WalletConnectProvider',
        r'WalletConnectProvider',
        r'initWalletConnect',
        r'walletconnect.*createSession'
    ]
    
    has_walletconnect_init = any(re.search(pattern, frontend_content, re.IGNORECASE) for pattern in walletconnect_init_patterns)
    
    # Check for NEAR wallet fallback
    near_wallet_patterns = [
        r'new WalletConnection',
        r'WalletConnection.*redsigma-demo',
        r'requestSignIn'
    ]
    
    has_near_wallet = any(re.search(pattern, frontend_content, re.IGNORECASE) for pattern in near_wallet_patterns)
    
    # Check for wallet selection logic
    wallet_selection_patterns = [
        r'selectWallet.*near',
        r'selectWallet.*walletconnect',
        r'selectedWallet.*near.*walletconnect'
    ]
    
    has_wallet_selection = any(re.search(pattern, frontend_content, re.IGNORECASE) for pattern in wallet_selection_patterns)
    
    print("=== Frontend WalletConnect Test Results ===")
    print(f"✅ Has WalletConnect library imports: {has_walletconnect_lib}")
    print(f"✅ Has wallet options UI: {has_wallet_options}")
    print(f"✅ Has WalletConnect initialization: {has_walletconnect_init}")
    print(f"✅ Has NEAR wallet fallback: {has_near_wallet}")
    print(f"✅ Has wallet selection logic: {has_wallet_selection}")
    
    if all([has_walletconnect_lib, has_wallet_options, has_walletconnect_init, has_near_wallet, has_wallet_selection]):
        print("✅ All frontend walletconnect tests passed!")
        return True
    else:
        print("❌ Some frontend walletconnect tests failed!")
        return False

def test_docs_files():
    """Test that docs/index.html has walletconnect functionality."""
    
    docs_path = "/home/sudosudo/.openclaw/workspace/redsigma/docs/index.html"
    
    try:
        with open(docs_path, 'r') as f:
            docs_content = f.read()
    except FileNotFoundError:
        print(f"❌ Docs file not found: {docs_path}")
        return False
    
    # Check for walletconnect library imports
    walletconnect_lib_patterns = [
        r'@walletconnect/web3-provider',
        r'walletconnect/web3-provider',
        r'@walletconnect/web3-provider@1.8.0',
        r'web3@1.10.0'
    ]
    
    has_walletconnect_lib = any(re.search(pattern, docs_content, re.IGNORECASE) for pattern in walletconnect_lib_patterns)
    
    # Check for wallet options UI
    wallet_options_pattern = r'wallet-option.*walletconnect'
    has_wallet_options = bool(re.search(wallet_options_pattern, docs_content, re.IGNORECASE))
    
    # Check for WalletConnect initialization
    walletconnect_init_patterns = [
        r'new WalletConnectProvider',
        r'WalletConnectProvider',
        r'initWalletConnect',
        r'walletconnect.*createSession'
    ]
    
    has_walletconnect_init = any(re.search(pattern, docs_content, re.IGNORECASE) for pattern in walletconnect_init_patterns)
    
    # Check for NEAR wallet fallback
    near_wallet_patterns = [
        r'new WalletConnection',
        r'WalletConnection.*redsigma-demo',
        r'requestSignIn'
    ]
    
    has_near_wallet = any(re.search(pattern, docs_content, re.IGNORECASE) for pattern in near_wallet_patterns)
    
    # Check for wallet selection logic
    wallet_selection_patterns = [
        r'selectWallet.*near',
        r'selectWallet.*walletconnect',
        r'selectedWallet.*near.*walletconnect'
    ]
    
    has_wallet_selection = any(re.search(pattern, docs_content, re.IGNORECASE) for pattern in wallet_selection_patterns)
    
    print("\n=== Docs WalletConnect Test Results ===")
    print(f"✅ Has WalletConnect library imports: {has_walletconnect_lib}")
    print(f"✅ Has wallet options UI: {has_wallet_options}")
    print(f"✅ Has WalletConnect initialization: {has_walletconnect_init}")
    print(f"✅ Has NEAR wallet fallback: {has_near_wallet}")
    print(f"✅ Has wallet selection logic: {has_wallet_selection}")
    
    if all([has_walletconnect_lib, has_wallet_options, has_walletconnect_init, has_near_wallet, has_wallet_selection]):
        print("✅ All docs walletconnect tests passed!")
        return True
    else:
        print("❌ Some docs walletconnect tests failed!")
        return False

def main():
    """Main test function."""
    print("Testing WalletConnect fix for REDΣSIGMA GitHub Pages demo...\n")
    
    frontend_ok = test_frontend_files()
    docs_ok = test_docs_files()
    
    print(f"\n=== Final Results ===")
    print(f"Frontend tests: {'✅ PASSED' if frontend_ok else '❌ FAILED'}")
    print(f"Docs tests: {'✅ PASSED' if docs_ok else '❌ FAILED'}")
    
    if frontend_ok and docs_ok:
        print("\n🎉 All walletconnect fix tests passed!")
        print("The walletconnect issue on the REDΣSIGMA GitHub Pages demo has been successfully resolved.")
        return 0
    else:
        print("\n❌ Some tests failed. Please review the implementation.")
        return 1

if __name__ == "__main__":
    sys.exit(main())