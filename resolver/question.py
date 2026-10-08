import json
import re
import time
from functools import lru_cache
from html import unescape
from typing import Callable, List, Optional

from rich import errors
from rich.console import Console, ConsoleOptions, Group, RenderableType, RenderResult
from rich.json import JSON
from rich.layout import Layout
from rich.panel import Panel
from rich.protocol import is_renderable
from rich.style import StyleType
from rich.styled import Styled
from rich.table import Column, Row, Table
from rich.text import Text

import config
from cxapi.base import QAQDtoBase
from cxapi.exception import APIError
from cxapi.schema import (
    SUPPORTED_QUESTION_TYPES,
    QuestionModel,
    QuestionsExportSchema,
    QuestionsExportType,
    QuestionType,
    is_valid_answer,
)
from cxapi.utils import normalize_text
from logger import Logger

from .searcher import MultiSearcherWraper, SearcherResp
from .searcher.json import JsonFileSearcher
from .searcher.ollama import OllamaSearcherAPI
from .searcher.openai import OpenAISearcher
from .searcher.restapi import (
    CxSearcher,
    EnncySearcher,
    JsonApiSearcher,
    LemonSearcher,
    LyCk6Searcher,
    MukeSearcher,
    RestApiSearcher,
    TiKuHaiSearcher,
)
from .searcher.sqlite import SqliteSearcher

# 所有的搜索器类
SEARCHERS = {
    "JsonFileSearcher": JsonFileSearcher,
    "CxSearcher": CxSearcher,
    "EnncySearcher": EnncySearcher,
    "RestApiSearcher": RestApiSearcher,
    "SqliteSearcher": SqliteSearcher,
    "TiKuHaiSearcher": TiKuHaiSearcher,
    "LyCk6Searcher": LyCk6Searcher,
    "MukeSearcher": MukeSearcher,
    "JsonApiSearcher": JsonApiSearcher,
    "LemonSearcher": LemonSearcher,
    "OpenAISearcher": OpenAISearcher,
    "OllamaSearcherAPI": OllamaSearcherAPI,
}


def _has_analysis(text: str) -> bool:
    return (
        re.search(
            r"(?:^|[,，;；。.\s])(?:解析|解释|理由|说明|analysis|explanation|reason)\s*[:：]",
            normalize_text(text),
            flags=re.IGNORECASE,
        )
        is not None
    )


def _unique_json_object(pairs):
    fields = {}
    for key, value in pairs:
        if key in fields:
            raise ValueError("答案字段重复")
        fields[key] = value
    return fields


def _has_thinking(text: str) -> bool:
    return re.search(r"</?think\b", unescape(text), flags=re.IGNORECASE) is not None


def _answer_field(answer):
    if isinstance(answer, str):
        answer = unescape(answer).strip()
        if answer.casefold().startswith("<think>"):
            thinking = re.match(r"<think>(.*?)</think>\s*", answer, flags=re.IGNORECASE | re.DOTALL)
            if thinking is None or _has_thinking(thinking[1]):
                return None
            answer = answer[thinking.end() :]
        if _has_thinking(answer):
            return None
        if "```" in answer:
            fenced = re.fullmatch(
                r"```(?:json|text)?[ \t]*\r?\n(.*?)\r?\n```[ \t]*",
                answer,
                flags=re.IGNORECASE | re.DOTALL,
            )
            if fenced is None or "```" in fenced[1]:
                return None
            answer = fenced[1].strip()
        if answer.startswith(("{", "[", '"')):
            try:
                answer = json.loads(answer, object_pairs_hook=_unique_json_object)
            except (ValueError, TypeError, RecursionError):
                return None
    # JSON 题库保留对象的字段对，以免重复键被悄悄覆盖。
    if isinstance(answer, tuple):
        try:
            answer = _unique_json_object(answer)
        except (ValueError, TypeError):
            return None
    if isinstance(answer, dict):
        fields = [key for key in ("answer", "answers", "答案") if key in answer]
        if len(fields) != 1:
            return None
        answer = answer[fields[0]]
    if isinstance(answer, str):
        if _has_thinking(answer) or "```" in answer:
            return None
        answer = re.split(
            r"\n\s*(?:解析|解释|理由|说明|analysis|explanation|reason)\s*[:：]",
            answer,
            maxsplit=1,
            flags=re.IGNORECASE,
        )[0]
        answer = re.sub(
            r"^\s*(?:参考答案|正确答案|答案|answer)\s*[:：]\s*",
            "",
            answer,
            count=1,
            flags=re.IGNORECASE,
        )
        if _has_analysis(answer):
            return None
    elif isinstance(answer, list) and any(
        isinstance(value, str) and (_has_thinking(value) or "```" in value) for value in answer
    ):
        return None
    return answer


def _label(text: str) -> str:
    return text.translate(
        str.maketrans(
            "ＡＢＣＤＥＦＧＨＩＪＫＬＭＮＯＰＱＲＳＴＵＶＷＸＹＺａｂｃｄｅｆｇｈｉｊｋｌｍｎｏｐｑｒｓｔｕｖｗｘｙｚ",
            "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz",
        )
    ).upper()


def _option_text(key: str, value: str) -> str:
    text = normalize_text(value)
    prefix = re.match(r"^([A-Za-zＡ-Ｚａ-ｚ])[.．、:：)）]\s*", text)
    if prefix and _label(prefix[1]) == _label(key):
        text = text[prefix.end() :]
    return text


def _choice(question: QuestionModel, answer: str) -> str | None:
    text = normalize_text(answer)
    if not text or not isinstance(question.options, dict):
        return None
    labels = [key for key in question.options if _label(key) == _label(text)]
    prefix = re.match(r"^([A-Za-zＡ-Ｚａ-ｚ])[.．、:：)）]\s*(.*)$", text)
    if prefix:
        keys = [key for key in question.options if _label(key) == _label(prefix[1])]
        if len(keys) != 1:
            return None
        key = keys[0]
        body = normalize_text(prefix[2])
        return key if not body or body == _option_text(key, question.options[key]) else None
    matches = set(labels)
    matches.update(
        key for key, value in question.options.items() if _option_text(key, value) == text
    )
    return next(iter(matches)) if len(matches) == 1 else None


def normalize_answer(question: QuestionModel, answer) -> str | list[str] | bool | None:
    """Return only a complete, unambiguous answer in the DTO's existing format."""
    if question.type not in SUPPORTED_QUESTION_TYPES:
        return None
    # 数学区间、集合也可能是 JSON；两种解释不同则拒绝猜测。
    if (
        isinstance(answer, str)
        and answer.strip().startswith(("[", "{"))
        and question.type in (QuestionType.单选题, QuestionType.多选题)
    ):
        literal = _choice(question, answer)
        if literal is not None:
            candidate = normalize_answer(question, _answer_field(answer))
            return literal if candidate is None or candidate == literal else None
    answer = _answer_field(answer)
    if question.type == QuestionType.判断题:
        if type(answer) is bool:
            return answer
        if type(answer) is int and answer in (0, 1):
            return bool(answer)
        if not isinstance(answer, str):
            return None
        text = normalize_text(answer).strip("。.").casefold()
        if text in {"对", "正确", "是", "√", "true", "yes", "1"}:
            return True
        if text in {"错", "错误", "否", "不正确", "不对", "不是", "×", "✗", "false", "no", "0"}:
            return False
        return None
    if question.type == QuestionType.填空题:
        if isinstance(answer, str):
            answer = answer.split("#")
        if (
            not isinstance(question.options, list)
            or not question.options
            or not isinstance(answer, list)
            or len(answer) != len(question.options)
            or any(
                not isinstance(value, str) or not normalize_text(value) or _has_analysis(value)
                for value in answer
            )
        ):
            return None
        blanks = [normalize_text(value) for value in answer]
        if isinstance(question.answer, list):
            if len(question.answer) != len(blanks):
                return None
            for index, previous in enumerate(question.answer):
                if isinstance(previous, str) and normalize_text(previous):
                    if normalize_text(previous) != blanks[index]:
                        return None
                    blanks[index] = previous
        return blanks
    if isinstance(answer, list):
        if not answer or any(not isinstance(value, str) for value in answer):
            return None
        parts = answer
    elif isinstance(answer, str):
        if question.type == QuestionType.单选题:
            return _choice(question, answer)
        whole = _choice(question, answer)
        parts = re.split(r"[#;；,，、|\n]+", answer)
        compact = _label(re.sub(r"\s+", "", answer))
        if len(parts) == 1 and re.fullmatch(r"[A-Z]+", compact):
            parts = list(compact)
        keys = [_choice(question, value) for value in parts]
        if whole is not None:
            if all(key is not None for key in keys) and set(keys) != {whole}:
                return None
            return whole
    else:
        return None
    if question.type == QuestionType.单选题:
        return _choice(question, parts[0]) if len(parts) == 1 else None
    if not isinstance(question.options, dict):
        return None
    keys = [_choice(question, value) for value in parts]
    if not keys or any(key is None for key in keys):
        return None
    return "".join(key for key in question.options if key in keys)


@lru_cache(maxsize=128)
def load_searcher() -> MultiSearcherWraper:
    """加载搜索器实例 缓存最终加载结果
    Returns:
        MultiSearcherWraper: 多搜索器封装
    """
    searcher = MultiSearcherWraper()
    # 检查题库后端配置
    if not config.SEARCHERS:
        raise AttributeError("请先配置题库后端再运行，如不需要使用答题功能请修改config.yml进行关闭。")
    # 按需实例化并添加搜索器
    for searcher_conf in config.SEARCHERS:
        typename = searcher_conf["type"]
        typename = typename[0].upper() + typename[1:]
        if typename not in SEARCHERS:
            raise AttributeError(f'Searcher "{typename}" not found')
        # 动态加载搜索器类
        searcher.add(SEARCHERS[typename](**{k: v for k, v in searcher_conf.items() if k != "type"}))

    return searcher


class MyTable(Table):
    def push_row(
        self,
        *renderables: Optional["RenderableType"],
        style: Optional[StyleType] = None,
    ) -> None:
        """向表格顶部插入行"""

        def add_cell(column: Column, renderable: "RenderableType") -> None:
            column._cells.insert(0, renderable)

        cell_renderables: List[Optional["RenderableType"]] = list(renderables)

        columns = self.columns
        if len(cell_renderables) < len(columns):
            cell_renderables = [
                *cell_renderables,
                *[None] * (len(columns) - len(cell_renderables)),
            ]
        for index, renderable in enumerate(cell_renderables):
            if index == len(columns):
                column = Column(_index=index)
                for _ in self.rows:
                    add_cell(column, Text(""))
                self.columns.append(column)
            else:
                column = columns[index]
            if renderable is None:
                add_cell(column, "")
            elif is_renderable(renderable):
                add_cell(column, renderable)
            else:
                raise errors.NotRenderableError(
                    f"unable to render {type(renderable).__name__}; a string or other renderable object is required"
                )
        self.rows.insert(0, Row(style=style))


class SearchRespShowComp:
    """搜索结果展示组件
    用于 TUI 显示
    """

    question: QuestionModel  # 题目
    results: list[SearcherResp]  # 搜索返回

    def __init__(self, question: QuestionModel, results: list[SearcherResp]) -> None:
        self.question = question
        self.results = results

    def __rich_console__(self, console: Console, options: ConsoleOptions) -> RenderResult:
        """rich 渲染接口"""
        yield Group(Text("q: ", end=""), Text(self.question.value, style="cyan"))
        for result in self.results:
            yield Group(
                Text("a: ", end=""),
                Styled(
                    Group(
                        Text(f"{result.searcher.__class__.__name__}", end=" "),
                        Text(
                            "Ok" if result.code == 0 else f"Err {result.code}:{result.message}",
                            end="",
                        ),
                        Text(" -> " if result.code == 0 else "", end=""),
                    ),
                    style="green" if result.code == 0 else "red",
                ),
                Text(str(result.answer), style="cyan", overflow="ellipsis")
                if result.code == 0
                else "",
            )


class QuestionResolver:
    """题目解决器
    用于 拉取-搜索-填充-提交 工作流的自动接管
    """

    searcher: MultiSearcherWraper  # 搜索器
    logger: Logger  # 日志记录器
    exam_dto: QAQDtoBase  # 实例化的答题接口对象
    enable_fallback_save: bool  # 是否失败时保存
    enable_fallback_fuzzer: bool  # 兼容旧调用，严格匹配始终关闭
    persubmit_delay: float  # 每次提交的延迟
    auto_final_submit: bool  # 是否自动交卷
    cb_confirm_submit: Callable[[int, int, list, QAQDtoBase], bool]  # 交卷确认回调函数

    tui_ctx: Layout

    mistakes: list[tuple[QuestionModel, str]]  # 未完成题目
    completed_cnt: int  # 已答题计数
    incompleted_cnt: int  # 未答题计数
    finish_flag: bool  # 答题完毕标志

    def __init__(
        self,
        exam_dto: QAQDtoBase,
        fallback_save: bool = True,
        fallback_fuzzer: bool = False,
        persubmit_delay: float = 1.0,
        auto_final_submit: bool = True,
        cb_confirm_submit: Callable[[int, int, list, QAQDtoBase], bool] = None,
    ) -> None:
        """constructor
        Args:
            exam_dto: 答题接口对象
            fallback_save: 是否失败时保存
            fallback_fuzzer: 兼容旧调用，严格匹配忽略此选项
            persubmit_delay: 每次提交的延迟
            auto_final_submit: 是否自动交卷
            cb_confirm_submit： 交卷确认回调函数(completed_cnt, incompleted_cnt, mistakes, exam_dto)
        """
        self.logger = Logger("QuestionResolver")
        self.exam_dto = exam_dto
        self.enable_fallback_save = fallback_save
        self.enable_fallback_fuzzer = False
        if fallback_fuzzer:
            self.logger.warning("严格匹配已忽略 fallback_fuzzer，不填入随机答案")
        self.persubmit_delay = persubmit_delay
        self.auto_final_submit = auto_final_submit
        self.cb_confirm_submit = cb_confirm_submit
        self.searcher = None
        self.last_match_error = ""

        self.tui_ctx = Layout(name="Resolver")  # 当前类所属 TUI 的 ctx
        self.mistakes = []
        self.completed_cnt = 0
        self.incompleted_cnt = 0
        self.finish_flag = False

    def __rich_console__(self, console: Console, options: ConsoleOptions) -> RenderResult:
        yield self.tui_ctx

    def fill(self, question: QuestionModel, search_results: list[SearcherResp]) -> bool:
        "查询并填充对应选项"
        self.last_match_error = ""
        if is_valid_answer(question):
            return True
        if any(result.code == -409 for result in search_results):
            self.last_match_error = "题库答案冲突"
            return False
        candidates = []
        for result in search_results:
            if result.code != 0:
                continue
            answer = normalize_answer(question, result.answer)
            if answer is not None and answer not in candidates:
                candidates.append(answer)
        if len(candidates) != 1:
            self.last_match_error = "候选答案冲突" if candidates else "答案未完整、唯一匹配"
            self.logger.warning(self.last_match_error)
            return False
        question.answer = candidates[0]
        return True

    def logging_mistake(self) -> None:
        """记录未完成题目到日志"""
        incomplete_msg = []

        incomplete_msg.append(f"\n-----*共 {self.incompleted_cnt} 题未完成*-----")
        for index, (q, a) in enumerate(self.mistakes, 1):
            incomplete_msg.append(f"{index}.\tq({q.type.name}/{q.type.value}): {q.value}")
            if q.type in (QuestionType.单选题, QuestionType.多选题):
                incomplete_msg.append("\to: " + " ".join(f"{a}={o}" for a, o in q.options.items()))
            incomplete_msg.append(f"\ta: {a}")
        incomplete_msg.append("------------")

        self.logger.warning("\n".join(incomplete_msg))

    def save_mistake(self) -> None:
        """保存未完成题目到文件"""
        schema = QuestionsExportSchema(
            id=0,
            title=self.exam_dto.title,
            type=QuestionsExportType.Mistakes,
            questions=[q for q, a in self.mistakes],
        )
        export_path = config.EXPORT_PATH / f"mistakes_{int(time.time())}.json"
        with export_path.open("w", encoding="utf8") as fp:
            fp.write(schema.to_json(ensure_ascii=False, separators=(",", ":")))

    def reg_confirm_submit_cb(self, cb: Callable[[int, int, list, QAQDtoBase], bool]):
        """注册提交确认函数"""
        self.cb_confirm_submit = cb
        return cb

    def execute(self) -> None:
        """执行自动接管逻辑"""
        self.completed_cnt = 0
        self.incompleted_cnt = 0
        self.mistakes = []
        self.finish_flag = False
        self.logger.info(f"开始处理试题 {self.exam_dto}")
        msg_console = Layout(name="Message", size=9)
        tb = MyTable("题号 / id", "类型", "题目", "答案", expand=True, border_style="yellow")
        self.tui_ctx.split_column(tb, msg_console)

        def refresh_title():
            state = (
                f"[bold red]{self.incompleted_cnt} 题未完成[/]"
                if self.finish_flag and self.incompleted_cnt
                else "[bold green]答题完毕[/]"
                if self.finish_flag
                else "[yellow]答题中[/]"
            )
            tb.title = f"{state}  {self.exam_dto.title}  [green]{self.completed_cnt}[/]/[red]{self.incompleted_cnt}"

        refresh_title()
        seen_ids = set()
        for index, question in self.exam_dto:
            results = []
            reason = ""
            status = is_valid_answer(question)
            previous_answer = question.answer
            if question.id in seen_ids:
                status = False
                reason = "题目 ID 重复"
            elif status:
                self.completed_cnt += 1
                self.logger.info(f"保留已有答案 ({index})")
            elif question.type not in SUPPORTED_QUESTION_TYPES:
                reason = f"不支持题型 {question.type.name}"
            else:
                try:
                    if self.searcher is None:
                        self.searcher = load_searcher()
                    results = self.searcher.invoke(question)
                    msg_console.update(Panel(SearchRespShowComp(question, results), title="搜索器返回"))
                    status = self.fill(question, results)
                    reason = self.last_match_error
                except Exception as err:
                    reason = f"搜索失败: {type(err).__name__}"
                    status = False
                if status:
                    time.sleep(self.persubmit_delay)
                    try:
                        result = self.exam_dto.submit(index=index, question=question)
                        if not isinstance(result, dict):
                            raise APIError("无效的单题回执")
                        cached = (
                            result.get("index") == index
                            and result.get("question") == question.value
                            and result.get("answer") == question.answer
                            and result.get("status", True) is True
                        )
                        if result.get("status") != "success" and not cached:
                            raise APIError("单题回执未确认成功")
                    except Exception as err:
                        status = False
                        question.answer = previous_answer
                        reason = f"提交失败: {type(err).__name__}"
                        self.logger.warning(reason)
                        msg_console.update(Panel(reason, title="提交失败", border_style="red"))
                    else:
                        self.completed_cnt += 1
                        receipt_title = "答案已缓存" if cached else "答案提交成功"
                        self.logger.info(receipt_title)
                        msg_console.update(
                            Panel(
                                JSON.from_data(result, ensure_ascii=False),
                                title=receipt_title,
                                border_style="green",
                            )
                        )
            seen_ids.add(question.id)
            if not status:
                self.incompleted_cnt += 1
                self.mistakes.append((question, reason))
            tb.push_row(
                f"[green]{index + 1}[/] ({question.id})",
                question.type.name,
                question.value,
                f"[green]{question.answer}" if status else f"[red]{reason}",
            )
            refresh_title()

        parse_errors = getattr(self.exam_dto, "parse_errors", {})
        self.incompleted_cnt += len(parse_errors)
        for index, reason in parse_errors.items():
            self.logger.warning(f"第 {index + 1} 题解析未完成: {reason}")
        if not self.completed_cnt and not self.incompleted_cnt:
            self.incompleted_cnt = 1
            self.logger.warning("未解析到题目，禁止交卷")
        self.finish_flag = True
        refresh_title()
        tb.border_style = "red" if self.incompleted_cnt else "green"

        if self.incompleted_cnt:
            msg_console.update(
                Panel(f"{self.incompleted_cnt} 题未完成，请查看日志", title="试题未完成", border_style="red")
            )
            self.logging_mistake()
            self.save_mistake()
            if self.enable_fallback_save:
                try:
                    result = self.exam_dto.fallback_save()
                    if (
                        not isinstance(result, dict)
                        or result.get("status") is not True
                        or result.get("msg") == "NotImplemented!"
                    ):
                        raise APIError("临时保存回执未确认成功")
                except Exception as err:
                    self.logger.warning(f"临时保存未确认: {type(err).__name__}")
                    msg_console.update(Panel("临时保存未确认", title="保存失败", border_style="red"))
                else:
                    self.logger.info("临时保存成功")
                    msg_console.update(
                        Panel(
                            JSON.from_data(result, ensure_ascii=False),
                            title="临时保存成功",
                            border_style="green",
                        )
                    )
            return
        if not self.auto_final_submit:
            return
        if self.cb_confirm_submit is not None and not self.cb_confirm_submit(
            self.completed_cnt, self.incompleted_cnt, self.mistakes, self.exam_dto
        ):
            return
        try:
            result = self.exam_dto.final_submit()
            if not isinstance(result, dict) or not (
                result.get("status") is True or result.get("status") == "success"
            ):
                raise APIError("交卷回执未确认成功")
        except Exception as err:
            self.logger.warning(f"交卷未确认: {type(err).__name__}")
            msg_console.update(Panel("交卷未确认", title="交卷失败", border_style="red"))
        else:
            self.logger.info("交卷成功")
            msg_console.update(
                Panel(
                    JSON.from_data(result, ensure_ascii=False), title="交卷成功", border_style="green"
                )
            )


__all__ = ["QuestionResolver"]
