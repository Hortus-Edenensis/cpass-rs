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
        self.config = config
        self.client = OpenAI(api_key=config["api_key"], base_url=config["base_url"])
        self.logger = Logger("OpenAISearcher")

    def invoke(self, question: QuestionModel) -> SearcherResp:

        # 将选项从JSON转换成人类(GPT)易读形式
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
            response = self.client.chat.completions.create(
                model=self.config["model"],
                temperature=0.5,  # 答题场景适合把temperature调低
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

            response = response.choices[0].message.content
        except Exception as err:
            return SearcherResp(-500, f"搜索器请求失败 ({type(err).__name__})", self, question.value, None)

        self.logger.info(f"返回结果：{response}")
        return SearcherResp(0, "", self, question.value, response)
