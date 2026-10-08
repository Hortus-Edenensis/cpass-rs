"""Offline checks for the Python OpenAI-compatible searcher and SDK request body."""

import importlib
import json
import sys
import unittest
from pathlib import Path
from types import ModuleType, SimpleNamespace
from unittest.mock import MagicMock, patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
for name in ("cxapi", "resolver"):
    if name not in sys.modules:
        package = ModuleType(name)
        package.__path__ = [str(ROOT.joinpath(*name.split(".")))]
        sys.modules[name] = package
if "logger" not in sys.modules:
    logger = ModuleType("logger")
    logger.Logger = lambda *args, **kwargs: MagicMock()
    sys.modules["logger"] = logger

import httpx
from openai import OpenAI

from cxapi.schema import QuestionModel, QuestionType

searcher_module = importlib.import_module("resolver.searcher.openai")


def completion(content="A", finish_reason="stop", reasoning_content="B"):
    return SimpleNamespace(
        choices=[
            SimpleNamespace(
                finish_reason=finish_reason,
                message=SimpleNamespace(content=content, reasoning_content=reasoning_content),
            )
        ]
    )


class OpenAISearcherTests(unittest.TestCase):
    def setUp(self):
        self.client_factory = patch.object(searcher_module, "OpenAI").start()
        self.addCleanup(patch.stopall)
        patch("socket.create_connection", side_effect=AssertionError("network forbidden")).start()
        self.client = self.client_factory.return_value
        self.client.chat.completions.create.return_value = completion()
        self.settings = {
            "base_url": "https://api.deepseek.com",
            "model": "deepseek-v4.1-flash",
            "api_key": "sk-offline-test",
            "prompt": "{type}：{value}\n{options}",
            "system_prompt": "只回答答案",
        }
        self.question = QuestionModel(42, "以下不正确的是？", QuestionType.单选题, {"A": "甲", "B": "乙"}, None)

    def searcher(self, **overrides):
        return searcher_module.OpenAISearcher(**(self.settings | overrides))

    def test_deepseek_controls_use_sdk_extra_body(self):
        instance = self.searcher(
            thinking={"type": "enabled"},
            reasoning_effort="ultra",
            max_tokens=32768,
            response_format={"type": "json_object"},
        )
        self.assertEqual(instance.invoke(self.question).answer, "A")
        request = self.client.chat.completions.create.call_args.kwargs
        self.assertEqual(request["model"], "deepseek-flash")
        self.assertEqual(
            request["extra_body"],
            {
                "thinking": {"type": "enabled"},
                "reasoning_effort": "max",
            },
        )
        self.assertNotIn("reasoning_effort", request)
        self.assertEqual(request["max_tokens"], 32768)
        self.assertEqual(request["response_format"], {"type": "json_object"})
        self.assertIn('{"answer":"A"}', request["messages"][0]["content"])
        self.assertIn("以下不正确的是？", request["messages"][1]["content"])
        self.assertNotIn("sk-offline-test", repr(request))

    def test_gateway_aliases_and_default_request_remain_unchanged(self):
        for base_url in (
            "https://gateway.example.com/v1",
            "https://api.deepseek.com.gateway.example.com/v1",
        ):
            with self.subTest(base_url=base_url):
                self.searcher(base_url=base_url).invoke(self.question)
                request = self.client.chat.completions.create.call_args.kwargs
                self.assertEqual(request["model"], "deepseek-v4.1-flash")
                self.assertEqual(set(request), {"model", "temperature", "messages"})
        for effort, mapped in (
            ("none", "none"),
            ("minimal", "low"),
            ("low", "low"),
            ("medium", "high"),
            ("high", "high"),
            ("xhigh", "high"),
            ("max", "max"),
            ("ultra", "max"),
        ):
            for base_url, expected in (
                ("https://api.deepseek.com/v1/", mapped),
                ("https://gateway.example.com/v1", effort),
            ):
                with self.subTest(base_url=base_url, effort=effort):
                    instance = self.searcher(base_url=base_url, reasoning_effort=effort)
                    instance.invoke(self.question)
                    request = self.client.chat.completions.create.call_args.kwargs
                    self.assertEqual(request["extra_body"]["reasoning_effort"], expected)
        self.searcher(thinking=None, reasoning_effort=None, max_tokens=None, response_format=None)

    def test_invalid_options_fail_before_client_construction(self):
        for field, value in (
            ("base_url", "ftp://api.deepseek.com"),
            ("base_url", "not-a-url"),
            ("base_url", "https://api.deepseek.com?api_key=secret"),
            ("base_url", "https://["),
            ("model", " "),
            ("api_key", None),
            ("prompt", 1),
            ("system_prompt", None),
            ("thinking", True),
            ("thinking", {"type": "auto"}),
            ("thinking", {"type": "enabled", "budget_tokens": 100}),
            ("reasoning_effort", "unknown"),
            ("reasoning_effort", 1),
            ("reasoning_effort", []),
            ("max_tokens", 0),
            ("max_tokens", -1),
            ("max_tokens", True),
            ("max_tokens", 1.5),
            ("max_tokens", "32768"),
            ("max_tokens", 393217),
            ("response_format", "json_object"),
            ("response_format", {"type": "json_schema"}),
        ):
            with self.subTest(field=field, value=value):
                with self.assertRaises(ValueError) as error:
                    self.searcher(**{field: value})
                self.assertIn(field, str(error.exception))
                self.assertNotIn("sk-offline-test", str(error.exception))
                self.client_factory.assert_not_called()
        for thinking, effort in (("enabled", "none"), ("disabled", "high")):
            with self.subTest(thinking=thinking, effort=effort):
                with self.assertRaisesRegex(ValueError, "conflict"):
                    self.searcher(thinking={"type": thinking}, reasoning_effort=effort)
                self.client_factory.assert_not_called()

    def test_valid_disabled_thinking_and_token_limits(self):
        instance = self.searcher(thinking={"type": "disabled"}, reasoning_effort="none")
        instance.invoke(self.question)
        self.assertEqual(
            self.client.chat.completions.create.call_args.kwargs["extra_body"],
            {
                "thinking": {"type": "disabled"},
                "reasoning_effort": "none",
            },
        )
        for base_url, max_tokens in (
            ("https://api.deepseek.com", 1),
            ("https://api.deepseek.com", 393216),
            ("https://gateway.example.com/v1", 393217),
        ):
            with self.subTest(base_url=base_url, max_tokens=max_tokens):
                instance = self.searcher(base_url=base_url, max_tokens=max_tokens)
                instance.invoke(self.question)
                self.assertEqual(
                    self.client.chat.completions.create.call_args.kwargs["max_tokens"], max_tokens
                )

    def test_reasoning_is_never_an_answer_and_raw_final_content_is_preserved(self):
        raw = '```json\n{"answer":"A"}\n```'
        instance = self.searcher()
        self.client.chat.completions.create.return_value = completion(raw, reasoning_content="B")
        self.assertEqual(instance.invoke(self.question).answer, raw)
        for content in (None, "", " ", [], {}, True, 1):
            with self.subTest(content=content):
                self.client.chat.completions.create.return_value = completion(content)
                result = instance.invoke(self.question)
                self.assertNotEqual(result.code, 0)
                self.assertIsNone(result.answer)
        self.client.chat.completions.create.return_value = SimpleNamespace(choices=[])
        self.assertIsNone(instance.invoke(self.question).answer)

    def test_incomplete_choices_are_rejected_and_empty_first_choice_is_skipped(self):
        instance = self.searcher()
        for reason in (
            "length",
            "content_filter",
            "tool_calls",
            "insufficient_system_resource",
            "aborted",
            "unknown",
        ):
            with self.subTest(finish_reason=reason):
                self.client.chat.completions.create.return_value = completion(finish_reason=reason)
                result = instance.invoke(self.question)
                self.assertNotEqual(result.code, 0)
                self.assertIsNone(result.answer)
        self.client.chat.completions.create.return_value = SimpleNamespace(
            choices=[
                completion(" ").choices[0],
                completion("B", "length").choices[0],
                completion().choices[0],
            ]
        )
        self.assertEqual(instance.invoke(self.question).answer, "A")
        self.client.chat.completions.create.return_value = SimpleNamespace(
            choices=[
                SimpleNamespace(message=SimpleNamespace(content="A")),
            ]
        )
        self.assertEqual(instance.invoke(self.question).answer, "A")

    def test_sdk_errors_do_not_leak_secrets(self):
        instance = self.searcher()
        self.client.chat.completions.create.side_effect = RuntimeError(
            "Authorization: Bearer sk-offline-test; response body contains secret"
        )
        result = instance.invoke(self.question)
        self.assertEqual(result.code, -500)
        self.assertIsNone(result.answer)
        self.assertIn("RuntimeError", result.message)
        self.assertNotIn("sk-offline-test", result.message)
        self.assertNotIn("Authorization", result.message)

    def test_real_sdk_serializes_controls_without_network(self):
        sent = []

        def handle(request):
            sent.append(json.loads(request.content))
            return httpx.Response(
                200,
                json={
                    "id": "offline",
                    "object": "chat.completion",
                    "created": 0,
                    "model": "deepseek-flash",
                    "choices": [
                        {
                            "index": 0,
                            "finish_reason": "stop",
                            "message": {
                                "role": "assistant",
                                "content": "A",
                                "reasoning_content": "B",
                            },
                        }
                    ],
                },
            )

        with httpx.Client(transport=httpx.MockTransport(handle)) as http_client:
            with OpenAI(
                api_key="sk-offline-test",
                base_url=self.settings["base_url"],
                http_client=http_client,
            ) as sdk:
                self.client_factory.return_value = sdk
                instance = self.searcher(
                    thinking={"type": "enabled"},
                    reasoning_effort="high",
                    max_tokens=32768,
                    response_format={"type": "json_object"},
                )
                self.assertEqual(instance.invoke(self.question).answer, "A")
        self.assertEqual(len(sent), 1)
        self.assertEqual(sent[0]["model"], "deepseek-flash")
        self.assertEqual(sent[0]["thinking"], {"type": "enabled"})
        self.assertEqual(sent[0]["reasoning_effort"], "high")
        self.assertEqual(sent[0]["max_tokens"], 32768)
        self.assertEqual(sent[0]["response_format"], {"type": "json_object"})
        self.assertNotIn("extra_body", sent[0])


if __name__ == "__main__":
    unittest.main()
