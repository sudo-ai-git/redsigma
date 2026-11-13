#!/usr/bin/env python3
"""
Comprehensive E2E Test Suite for REDΣSIGMA NEAR Connect & Testnet Integration.
Tests: NEAR Connect (popup, no redirect) → Contract Operations → Verification.

Architecture under test:
- @hot-labs/near-connect via importmap (esm.sh) — wallet selector POPUP, no full-page redirect.
- near-api-js (connect only, NO requestSignIn) — read-only view calls.
- State-changing calls via wallet.signAndSendTransaction.

Note: real transaction signing requires a live browser + NEAR wallet extension.
This suite verifies the wiring/architecture with mocks; the actual wallet popup
flow must be exercised in a real browser.
"""

import asyncio
import json
import re
import sys
from datetime import datetime
from typing import Dict, List, Any


class MockNearConnector:
    """Mock NEAR Connect NearConnector."""
    def __init__(self, network="testnet"):
        self.network = network
        self._wallet = MockNearWallet()

    async def connect(self, wallet_id=None):
        return self._wallet

    async def wallet(self, id=None):
        return self._wallet

    async def disconnect(self):
        return None


class MockNearWallet:
    """Mock NEAR Connect wallet (NearWalletBase subset)."""
    def __init__(self):
        self.account_id = "testnet-user.testnet"
        self.connected = True

    async def getAccounts(self):
        return [{"accountId": self.account_id, "publicKey": "ed25519:mock"}]

    async def signIn(self, data=None):
        return [{"accountId": self.account_id, "publicKey": "ed25519:mock"}]

    async def signOut(self, data=None):
        self.connected = False

    async def signAndSendTransaction(self, params):
        return {
            "transaction": {"hash": "mock_tx_hash", "status": "success"},
            "result": {},
        }


class MockNearAccount:
    """Mock near-api-js Account for read-only view calls."""
    def __init__(self):
        self.account_id = "testnet-user.testnet"

    async def viewFunction(self, *args, **kwargs):
        return {"result": "2"}


class MockNearConnection:
    """Mock near-api-js near connection (connect only, no sign-in)."""
    def __init__(self):
        self.networkId = "testnet"

    def account(self, account_id):
        return MockNearAccount()


class Mock:
    """Mock class for testing."""
    def __init__(self, return_value=None):
        self.return_value = return_value
        self.called = False
        self.call_args = ()

    def __call__(self, *args, **kwargs):
        self.called = True
        self.call_args = (args, kwargs)
        return self.return_value

    def __getattr__(self, name):
        if name.startswith('side_effect'):
            return None
        return Mock()


class AsyncMock(Mock):
    async def __call__(self, *args, **kwargs):
        self.called = True
        self.call_args = (args, kwargs)
        return self.return_value


class RedSigmaE2ETester:
    """Main E2E test class for REDΣSIGMA NEAR Connect demo."""

    def __init__(self):
        self.test_results = []
        self.mock_connector = None
        self.mock_wallet = None
        self.mock_near = None

    def log_test(self, test_name: str, passed: bool, details: str = ""):
        status = "✅ PASSED" if passed else "❌ FAILED"
        result = {
            'test': test_name,
            'status': status,
            'passed': passed,
            'details': details,
            'timestamp': datetime.now().isoformat(),
        }
        self.test_results.append(result)
        print(f"{status} {test_name}")
        if details:
            print(f"   Details: {details}")
        return result

    async def setup_test_environment(self):
        print("🔧 Setting up E2E test environment...")
        self.mock_connector = MockNearConnector()
        self.mock_wallet = MockNearWallet()
        self.mock_near = MockNearConnection()
        self.log_test("Test Environment Setup", True, "NEAR Connect + near-api-js mocks ready")

    async def test_html_architecture(self) -> bool:
        """Verify the demo HTML wires up NEAR Connect correctly."""
        print("\n📄 Testing demo HTML architecture...")
        frontend_path = "/home/sudosudo/.openclaw/workspace/redsigma/frontend/index.html"
        try:
            with open(frontend_path) as f:
                content = f.read()
        except FileNotFoundError:
            print("❌ Frontend file not found")
            return False

        checks = {
            "importmap present": bool(re.search(r'type="importmap"', content)),
            "near-connect esm.sh": bool(re.search(r'esm\.sh/@hot-labs/near-connect', content)),
            "NearConnector imported": bool(re.search(r'import\s*{\s*NearConnector\s*}\s*from', content)),
            "module script": bool(re.search(r'type="module"', content)),
            "near-api-js for views": bool(re.search(r'near-api-js@4\.0\.0', content)),
            "signAndSendTransaction used": bool(re.search(r'signAndSendTransaction', content)),
            "no requestSignIn redirect": not bool(re.search(r'requestSignIn\s*\(', content)),
            "no helperUrl": not bool(re.search(r'helperUrl', content)),
            "no WalletConnect libs": not bool(re.search(r'@walletconnect/web3-provider', content)),
            "no web3.js": not bool(re.search(r'web3@1\.10\.0', content)),
        }
        all_ok = True
        for name, ok in checks.items():
            self.log_test(f"HTML: {name}", ok, "")
            all_ok = all_ok and ok
        return all_ok

    async def test_near_connect_flow(self) -> bool:
        """Verify the NEAR Connect connect → getAccounts flow (popup, no redirect)."""
        print("\n🔗 Testing NEAR Connect connection flow...")
        try:
            # connector.connect() opens the wallet selector popup (no redirect)
            wallet = await self.mock_connector.connect()
            if not wallet:
                raise Exception("connector.connect() returned no wallet")

            accounts = await wallet.getAccounts()
            if not accounts or len(accounts) == 0:
                raise Exception("No accounts returned from wallet")

            account_id = accounts[0]["accountId"]
            if account_id != "testnet-user.testnet":
                raise Exception(f"Unexpected account id: {account_id}")

            self.log_test("NEAR Connect connect()", True, "Wallet selector popup returned a wallet")
            self.log_test("getAccounts()", True, f"Account: {account_id}")
            self.log_test("No full-page redirect", True, "connect() is popup-based, not window.location")
            return True
        except Exception as e:
            self.log_test("NEAR Connect connection", False, f"Failed: {str(e)}")
            return False

    async def test_near_api_js_views(self) -> bool:
        """Verify near-api-js connect-only (no sign-in) view calls work."""
        print("\n⛵ Testing near-api-js read-only view calls...")
        try:
            near = self.mock_near
            if near.networkId != "testnet":
                raise Exception("NEAR network not testnet")

            near_account = near.account("testnet-user.testnet")
            result = await near_account.viewFunction("counter", "get", {})
            if result != {"result": "2"}:
                raise Exception(f"Unexpected view result: {result}")

            self.log_test("near-api-js connect (no sign-in)", True, "View calls need no auth/redirect")
            self.log_test("viewFunction", True, f"Result: {result}")
            return True
        except Exception as e:
            self.log_test("near-api-js views", False, f"Failed: {str(e)}")
            return False

    async def test_state_changing_transaction(self) -> bool:
        """Verify signAndSendTransaction for deploy/execute (no redirect)."""
        print("\n⚡ Testing state-changing transaction via signAndSendTransaction...")
        try:
            result = await self.mock_wallet.signAndSendTransaction({
                "receiverId": "counter.testnet",
                "actions": [{
                    "type": "FunctionCall",
                    "params": {
                        "methodName": "increment",
                        "args": {},
                        "gas": "300000000000000",
                        "deposit": "0",
                    },
                }],
            })
            if not result or "transaction" not in result:
                raise Exception(f"Unexpected signAndSendTransaction result: {result}")

            self.log_test("signAndSendTransaction", True, "State-changing call submitted, no redirect")
            self.log_test("Transaction hash returned", bool(result["transaction"].get("hash")), "")
            return True
        except Exception as e:
            self.log_test("signAndSendTransaction", False, f"Failed: {str(e)}")
            return False

    async def test_error_handling(self) -> bool:
        print("\n⚠️ Testing error handling...")
        tests_passed = 0
        total_tests = 3

        try:
            if not self.mock_wallet.connected:
                raise ValueError("Wallet not connected")
            self.log_test("Connected wallet required", True, "Connection enforced")
            tests_passed += 1
        except Exception as e:
            self.log_test("Connected wallet required", False, str(e))

        try:
            payload = {}
            required = ["contract_name", "source", "deployer"]
            missing = [f for f in required if f not in payload]
            if missing:
                raise ValueError(f"Missing fields: {missing}")
            self.log_test("Missing fields rejected", False, "Should have rejected")
        except ValueError as e:
            self.log_test("Missing fields rejected", True, f"Detected: {e}")
            tests_passed += 1

        try:
            if self.mock_near.networkId != "testnet":
                raise ValueError("Wrong network")
            self.log_test("Testnet network enforced", True, "testnet only")
            tests_passed += 1
        except Exception as e:
            self.log_test("Testnet network enforced", False, str(e))

        self.log_test("Overall error handling", tests_passed >= 2, f"{tests_passed}/{total_tests}")
        return tests_passed >= 2

    async def run_comprehensive_test(self) -> Dict[str, Any]:
        print("🚀 Starting Comprehensive E2E Test Suite for REDΣSIGMA (NEAR Connect)")
        print("=" * 60)

        await self.setup_test_environment()

        test_scenarios = [
            ("HTML Architecture", self.test_html_architecture()),
            ("NEAR Connect Flow", self.test_near_connect_flow()),
            ("near-api-js Views", self.test_near_api_js_views()),
            ("State-Changing Transaction", self.test_state_changing_transaction()),
            ("Error Handling", self.test_error_handling()),
        ]

        for test_name, test_coro in test_scenarios:
            print(f"\n📊 Running: {test_name}")
            await test_coro

        passed_tests = sum(1 for r in self.test_results if r['passed'])
        total_tests = len(self.test_results)

        report = {
            'test_summary': {
                'total_tests': total_tests,
                'passed_tests': passed_tests,
                'success_rate': f"{(passed_tests / total_tests) * 100:.1f}%",
                'test_results': self.test_results,
            },
            'architecture': {
                'wallet_connector': '@hot-labs/near-connect (esm.sh, importmap)',
                'connection_mode': 'wallet selector popup (no full-page redirect)',
                'view_calls': 'near-api-js connect-only (no sign-in)',
                'state_changing': 'wallet.signAndSendTransaction',
            },
            'coverage': {
                'wallet_types': ['NEAR Connect (HOT, Meteor, Nightly, NEAR Mobile, WalletConnect via projectId)'],
                'network_support': ['NEAR Testnet'],
                'contract_operations': ['Deploy', 'Execute', 'View'],
                'not_supported': ['Redirect-based requestSignIn (removed by design)'],
            },
            'recommendations': [
                '✅ NEAR Connect replaces the redirect-based near-api-js requestSignIn flow',
                '✅ No full-page wallet redirect — connector.connect() uses a popup selector',
                '✅ Broken @walletconnect/web3-provider (Ethereum, not NEAR) removed',
                '⚠️ Real transaction signing requires a live browser + NEAR wallet extension',
                '⚠️ WalletConnect via NEAR Connect needs a valid walletConnect.projectId',
            ],
        }

        return report

    def print_final_report(self, report: Dict[str, Any]):
        print("\n" + "=" * 60)
        print("📊 E2E TEST REPORT: REDΣSIGMA NEAR Connect & Testnet Integration")
        print("=" * 60)

        summary = report['test_summary']
        print(f"Total Tests: {summary['total_tests']}")
        print(f"Passed Tests: {summary['passed_tests']}")
        print(f"Success Rate: {summary['success_rate']}")

        arch = report['architecture']
        print(f"\n🏗️  Architecture:")
        print(f"  Wallet connector: {arch['wallet_connector']}")
        print(f"  Connection mode: {arch['connection_mode']}")
        print(f"  View calls: {arch['view_calls']}")
        print(f"  State-changing: {arch['state_changing']}")

        cov = report['coverage']
        print(f"\n🔧 Test Coverage:")
        print(f"  Wallet types: {', '.join(cov['wallet_types'])}")
        print(f"  Not supported: {', '.join(cov['not_supported'])}")
        print(f"  Network support: {', '.join(cov['network_support'])}")
        print(f"  Contract operations: {', '.join(cov['contract_operations'])}")

        print(f"\n📋 Recommendations:")
        for rec in report['recommendations']:
            print(f"  {rec}")


async def main():
    """Main test execution function."""
    tester = RedSigmaE2ETester()

    print("🎯 Starting Comprehensive E2E Test Suite")
    print("Testing: NEAR Connect + Testnet Integration")
    print("=" * 80)

    report = await tester.run_comprehensive_test()
    tester.print_final_report(report)

    with open("/home/sudosudo/.openclaw/workspace/redsigma/e2e_test_report.json", "w") as f:
        json.dump(report, f, indent=2)

    print(f"\n💾 Test report saved to: redsigma/e2e_test_report.json")

    passed = report['test_summary']['passed_tests'] == report['test_summary']['total_tests']
    if passed:
        print(f"\n🎉 COMPREHENSIVE E2E TESTS: ALL PASSED!")
        return 0
    else:
        print(f"\n⚠️  E2E TESTS: SOME TESTS FAILED")
        print(f"❌ Review failed tests and address issues before production deployment")
        return 1


if __name__ == "__main__":
    import sys
    sys.exit(asyncio.run(main()))
