from urllib.parse import urlsplit

from openai import OpenAI

from cxapi.schema import QuestionModel
from logger import Logger

from . import SearcherBase, SearcherResp


class OpenAISearcher(SearcherBase):
    """ChatGPT 在线答题器"""

    client: OpenAI
    config: dict

    def __init__(self, **config) -> None:
        super().__init__()
        for field in ("base_url", "api_key", "model"):
            if not isinstance(config.get(field), str) or not config[field].strip():
                raise ValueError(f"{field} must be a non-empty string")
            config[field] = config[field].strip()
        try:
            endpoint = urlsplit(config["base_url"])
            valid_url = endpoint.scheme in ("http", "https") and endpoint.hostname
        except ValueError:
            valid_url = False
        if not valid_url or endpoint.query or endpoint.fragment:
            raise ValueError("base_url must be an absolute HTTP(S) URL without query or fragment")
        is_deepseek = endpoint.hostname == "api.deepseek.com"
        if is_deepseek and config["model"] == "deepseek-v4.1-flash":
            config["model"] = "deepseek-flash"
        for field, allowed in (
            ("thinking", ("enabled", "disabled")),
            ("response_format", ("text", "json_object")),
        ):
            value = config.get(field)
            if value is not None and (
                not isinstance(value, dict)
                or set(value) != {"type"}
                or value["type"] not in allowed
            ):
                raise ValueError(f"{field} must contain only a supported 'type'")
        effort = config.get("reasoning_effort")
        if effort is not None:
            if effort not in ("none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"):
                raise ValueError("reasoning_effort must be a supported effort string")
            thinking = config.get("thinking")
            if thinking and (thinking["type"] == "enabled") == (effort == "none"):
                raise ValueError("thinking and reasoning_effort conflict")
            if is_deepseek:
                config["reasoning_effort"] = {
                    "minimal": "low",
                    "medium": "high",
                    "xhigh": "high",
                    "ultra": "max",
                }.get(effort, effort)
        max_tokens = config.get("max_tokens")
        if max_tokens is not None and (
            type(max_tokens) is not int or max_tokens <= 0 or (is_deepseek and max_tokens > 393216)
        ):
            raise ValueError("max_tokens must be a positive integer within the model limit")
        config.setdefault("prompt", "{type}：{value}\n{options}")
        config.setdefault("system_prompt", "你是一位答题助手。请只输出答案本身，不要解释。")
        for field in ("prompt", "system_prompt"):
            if not isinstance(config[field], str) or not config[field].strip():
                raise ValueError(f"{field} must be a non-empty string")
        if (config.get("response_format") or {}).get("type") == "json_object":
            config["system_prompt"] += (
                '\n请输出 JSON 对象，例如 {"answer":"A"} 或 {"answer":["A","C"]}，' "只包含最终答案，不要解释。"
            )
        self.config = config
        self.client = OpenAI(api_key=config["api_key"], base_url=config["base_url"])
        self.logger = Logger("OpenAISearcher")

    def invoke(self, question: QuestionModel) -> SearcherResp:

        options_str = ""
        if question.options is not None:
            options_str = "选项：\n"
            if type(question.options) is dict:
                for k, v in question.options.items():
                    options_str += k + ". " + v + ";"
            elif type(question.options) is list:
                for v in question.options:
                    options_str += v + ";"

        self.logger.info(
            "从 "
            + self.config["prompt"]
            + " 生成提问："
            + str(self.config["prompt"]).format(
                type=question.type.name,
                value=question.value,
                options=options_str,
            ),
        )
        try:
            request = dict(
                model=self.config["model"],
                temperature=0.5,
                messages=[
                    {"role": "system", "content": self.config["system_prompt"]},
                    {
                        "role": "user",
                        "content": str(self.config["prompt"]).format(
                            type=question.type.name,
                            value=question.value,
                            options=options_str,
                        ),
                    },
                ],
            )
            extra_body = {
                key: self.config[key]
                for key in ("thinking", "reasoning_effort")
                if self.config.get(key) is not None
            }
            # The locked OpenAI 1.14.3 SDK has no reasoning_effort keyword parameter.
            if extra_body:
                request["extra_body"] = extra_body
            request.update(
                {
                    key: self.config[key]
                    for key in ("max_tokens", "response_format")
                    if self.config.get(key) is not None
                }
            )
            completion = self.client.chat.completions.create(**request)
            response = None
            for choice in completion.choices:
                if getattr(choice, "finish_reason", None) not in (None, "stop"):
                    continue
                content = getattr(choice.message, "content", None)
                if isinstance(content, str) and content.strip():
                    response = content
                    break
        except Exception as err:
            return SearcherResp(-500, f"搜索器请求失败 ({type(err).__name__})", self, question.value, None)

        if response is None:
            return SearcherResp(-500, "搜索器未返回完整答案", self, question.value, None)

        self.logger.info(f"返回结果：{response}")
        return SearcherResp(0, "", self, question.value, response)
