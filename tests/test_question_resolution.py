"""Offline regression checks; run: python -m unittest discover -s tests -v.

Uses the real parser/resolver/searcher modules and their normal lightweight dependencies.
Only application startup, logger, session, and captcha dependencies are isolated so the
suite does not require credentials, config.yml, OCR models, or writable runtime folders.
"""

import importlib
import json
import sys
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from types import ModuleType, SimpleNamespace
from unittest.mock import MagicMock, patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
for name in ("cxapi", "cxapi.task_point", "resolver"):
    package = ModuleType(name)
    package.__path__ = [str(ROOT.joinpath(*name.split(".")))]
    sys.modules[name] = package

config = ModuleType("config")
config.SEARCHERS = []
sys.modules["config"] = config
logger = ModuleType("logger")
logger.Logger = lambda *args, **kwargs: MagicMock()
sys.modules["logger"] = logger
session = ModuleType("cxapi.session")
session.SessionWraper = MagicMock
sys.modules["cxapi.session"] = session
captcha = ModuleType("cxapi.captcha.image")
captcha.ImageCaptchaDto = MagicMock
captcha.ImageCaptchaType = MagicMock()
captcha.fuck_slide_image_captcha = MagicMock()
sys.modules["cxapi.captcha.image"] = captcha

from bs4 import BeautifulSoup

from cxapi.base import QAQDtoBase
from cxapi.exception import APIError
from cxapi.schema import QuestionModel, QuestionType, is_valid_answer
from cxapi.utils import normalize_text
from resolver.searcher import MultiSearcherWraper, SearcherBase, SearcherResp
from resolver.searcher.json import JsonFileSearcher
from resolver.searcher.ollama import OllamaSearcherAPI
from resolver.searcher.openai import OpenAISearcher

resolver = importlib.import_module("resolver.question")
work = importlib.import_module("cxapi.task_point.work")
exam = importlib.import_module("cxapi.exam")


def question(kind=QuestionType.单选题, options=None, answer=None, value="测试题", qid=42):
    if options is None and kind in (QuestionType.单选题, QuestionType.多选题):
        options = {"A": "甲", "B": "乙", "C": "丙"}
    return QuestionModel(qid, value, kind, options, answer)


def response(answer, code=0):
    return SearcherResp(code, "test", None, "测试题", answer)


def work_html(type_value="0", title="单选题", qid="42", body=None, answer=""):
    body = body or "第一<span>行</span><p>第二行<br>第三行</p>"
    field = f'<input id="answertype{qid}" value="{type_value}">' if qid else ""
    return f"""<div class="Py-mian1">{field}
      <div class="Py-m1-title">1.（{title}，5.0分）{body}</div>
      <input class="answerInput" id="answer{qid}" value="{answer}">
      <ul><li class="more-choose-item"><em class="choose-opt" id-param="A">A.</em>
        <div class="choose-desc"><cc>甲<span>选项</span></cc></div></li>
      <li class="more-choose-item"><em class="choose-opt" id-param="B">B.</em>
        <div class="choose-desc">乙<span>选项</span></div></li></ul>
      <ul class="blankList2"><li><span>第1空</span><input class="blankInp2" value="已有"></li>
        <li><span>第2空</span><input class="blankInp2" value=""></li></ul>
    </div>"""


def exam_html(type_value="0", title="单选题", variant="answerMain", qid="42", answer=""):
    field = f'<input name="type{qid}" value="{type_value}">' if type_value is not None else ""
    identity = f'<input name="questionId" value="{qid}">' if qid else ""
    return f"""<div class="{variant} questionWrap singleQuesId ans-cc-exam" data="{qid}">
      {identity}<input name="typeName{qid}" value="3">{field}
      <div class="tit"><h3>{title}（5.0分）</h3>1.<span>（5.0分）</span>第一<span>行</span>
        <p>第二行<br>第三行</p></div>
      <input id="answer{qid}" value="{answer}">
      <div class="answerList radioList" name="A"><cc>甲<span>选项</span></cc></div>
      <div class="answerList radioList" name="B"><span>乙选项</span></div>
      <div class="completionList objectAuswerList"><span class="grayTit">第1空</span>
        <textarea class="blanktextarea">已有</textarea></div>
      <div class="completionList objectAuswerList"><span class="grayTit">第2空</span>
        <textarea class="blanktextarea"></textarea></div>
    </div>"""


def parse(parser, html):
    return parser(BeautifulSoup(html, "lxml").div)


class FakeDto(QAQDtoBase):
    def __init__(self, questions, receipt=None, parse_errors=None):
        super().__init__()
        self.title = "离线测验"
        self.questions = questions
        self.parse_errors = parse_errors or {}
        self.submit = MagicMock(
            return_value=receipt if receipt is not None else {"status": "success"}
        )
        self.final_submit = MagicMock(return_value={"status": "success"})
        self.fallback_save = MagicMock(return_value={"status": True})

    def __iter__(self):
        return iter(enumerate(self.questions))


class RegressionTests(unittest.TestCase):
    def setUp(self):
        folder = TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        self.folder = Path(folder.name)
        config.EXPORT_PATH = self.folder
        self.addCleanup(patch.stopall)
        patch("socket.create_connection", side_effect=AssertionError("network forbidden")).start()
        patch(
            "requests.sessions.Session.request", side_effect=AssertionError("network forbidden")
        ).start()
        patch.object(resolver.time, "sleep").start()

    def resolve(self, q, answers, **kwargs):
        instance = resolver.QuestionResolver(FakeDto([q]), **kwargs)
        return instance, instance.fill(q, [response(answer) for answer in answers])

    def execute(self, dto, answers=("A",), **kwargs):
        instance = resolver.QuestionResolver(dto, persubmit_delay=0, **kwargs)
        searcher = MagicMock()
        searcher.invoke.return_value = [response(answer) for answer in answers]
        with patch.object(resolver, "load_searcher", return_value=searcher):
            instance.execute()
        return instance, searcher

    def work_dto(self, html, total=1):
        dto = work.PointWorkDto(
            work_id="1",
            school_id="",
            job_id="2",
            session=MagicMock(),
            card_index=0,
            course_id=1,
            class_id=2,
            knowledge_id=3,
            cpi=4,
        )
        dto.ktoken = dto.enc = "test"
        fields = {
            "workAnswerId": 1,
            "totalQuestionNum": total,
            "workRelationId": 1,
            "fullScore": 10,
            "enc_work": "test",
        }
        inputs = "".join(f'<input id="{key}" value="{value}">' for key, value in fields.items())
        dto.session.get.return_value.text = (
            '<html><head><title>作业</title></head><body><h3 class="py-Title">离线作业</h3><form id="form1">'
            + inputs
            + html
            + "</form></body></html>"
        )
        return dto

    def exam_dto(self, html):
        dto = exam.ExamDto(
            session=MagicMock(),
            acc=SimpleNamespace(puid=1),
            exam_id=1,
            course_id=1,
            class_id=1,
            cpi=1,
            enc_task="test",
        )
        dto.title = "离线考试"
        dto.enc = "test"
        dto.enc_remain_time = dto.last_update_time = 0
        fields = {"enc": "test", "encRemainTime": 10, "remainTime": 10, "encLastUpdateTime": 10}
        inputs = "".join(f'<input id="{key}" value="{value}">' for key, value in fields.items())
        dto.session.get.return_value.text = (
            '<html><body><form id="submitTest">' + inputs + html + "</form></body></html>"
        )
        dto.refresh_tui = MagicMock()
        return dto

    def test_fetch_all_keeps_good_and_unsupported_questions_after_missing_id(self):
        for factory, html_factory in ((self.work_dto, work_html), (self.exam_dto, exam_html)):
            with self.subTest(dto=factory.__name__):
                dto = factory(
                    html_factory(qid="")
                    + html_factory(qid="43", type_value="999", title="新题型")
                    + html_factory(qid="44", answer="A"),
                    **({"total": 3} if factory == self.work_dto else {}),
                )
                questions = dto.fetch_all()
                self.assertEqual([q.id for q in questions], [43, 44])
                self.assertEqual(questions[0].type, QuestionType.其它)
                self.assertEqual(questions[1].answer, "A")
                self.assertIn(0, dto.parse_errors)
                with self.assertRaises(APIError):
                    dto.final_submit()
                dto.session.post.assert_not_called()

    def test_work_declared_missing_questions_prevent_final_submit(self):
        dto = self.work_dto(work_html(answer="A"), total=3)
        self.assertEqual(len(dto.fetch_all()), 1)
        self.assertTrue(dto.parse_errors)
        with self.assertRaises(APIError):
            dto.final_submit()
        dto.session.post.assert_not_called()

    def test_duplicate_ids_are_recorded_and_cannot_be_submitted(self):
        for factory, html_factory in ((self.work_dto, work_html), (self.exam_dto, exam_html)):
            with self.subTest(dto=factory.__name__):
                dto = factory(
                    html_factory(answer="A") + html_factory(answer="A"),
                    **({"total": 2} if factory == self.work_dto else {}),
                )
                questions = dto.fetch_all()
                ids = [q.id for q in questions]
                self.assertEqual(len(ids), len(set(ids)))
                self.assertTrue(dto.parse_errors)
                with self.assertRaises(APIError):
                    dto.final_submit()
                dto.session.post.assert_not_called()

    def test_work_iterator_keeps_page_indices_when_a_question_is_unparseable(self):
        dto = self.work_dto(work_html(qid="") + work_html(qid="44", answer="A"), total=2)
        rows = list(dto)
        self.assertEqual([(index, q.id) for index, q in rows], [(1, 44)])
        receipt = dto.submit(index=1, question=rows[0][1])
        self.assertEqual(receipt["index"], 1)
        self.assertEqual(dto.questions[0].answer, "A")

    def test_html_body_and_options_preserve_all_text(self):
        for parser, html in (
            (work.parse_question, work_html()),
            (exam.parse_question, exam_html()),
            (exam.parse_question, exam_html(variant="allAnswerList")),
        ):
            for content in (html, html.replace("\n", "")):
                with self.subTest(parser=parser.__module__, compressed="\n" not in content):
                    q = parse(parser, content)
                    self.assertEqual(q.type, QuestionType.单选题)
                    self.assertEqual(q.value, "第一行\n第二行\n第三行")
                    self.assertEqual(q.options, {"A": "甲选项", "B": "乙选项"})

    def test_type_id_wins_and_only_invalid_id_uses_heading(self):
        for parser, factory in ((work.parse_question, work_html), (exam.parse_question, exam_html)):
            for raw, title, expected in (
                ("0", "判断题", QuestionType.单选题),
                ("garbage", "判断题", QuestionType.判断题),
                ("garbage", "填空题", QuestionType.填空题),
                ("999", "填空题", QuestionType.其它),
                ("12", "单选题", QuestionType.其它),
                ("4", "判断题", QuestionType.简答题),
                ("999", "新题型", QuestionType.其它),
            ):
                with self.subTest(parser=parser.__module__, raw=raw, title=title):
                    self.assertEqual(
                        parse(parser, factory(type_value=raw, title=title)).type, expected
                    )
        self.assertEqual(
            parse(exam.parse_question, exam_html(type_value=None, title="判断题")).type,
            QuestionType.判断题,
        )

    def test_standalone_type_labels_are_removed_without_touching_body(self):
        for parser, html in (
            (work.parse_question, work_html()),
            (exam.parse_question, exam_html()),
        ):
            for tag_name in ("span", "strong", "b"):
                with self.subTest(parser=parser.__module__, label=tag_name):
                    node = BeautifulSoup(html, "lxml").div
                    title = node.select_one("div.Py-m1-title, div.tit")
                    title.clear()
                    fragment = BeautifulSoup(
                        f"<{tag_name}>单选题</{tag_name}>正文中单选题不删<p>下一行</p>", "lxml"
                    ).body
                    for child in list(fragment.contents):
                        title.append(child)
                    q = parser(node)
                    self.assertEqual(q.value, "正文中单选题不删\n下一行")

    def test_work_heading_fallback_with_separate_trusted_id(self):
        html = work_html(type_value="0", title="判断题").replace(
            '<input id="answertype42" value="0">', '<input name="questionId" value="42">'
        )
        self.assertEqual(parse(work.parse_question, html).type, QuestionType.判断题)

    def test_missing_trustworthy_id_is_parse_failure(self):
        for parser, html in (
            (work.parse_question, work_html(qid="")),
            (exam.parse_question, exam_html(qid="")),
        ):
            with self.subTest(parser=parser.__module__), self.assertRaises((ValueError, APIError)):
                parse(parser, html)

    def test_false_and_partial_blanks_survive_html(self):
        for parser, factory in ((work.parse_question, work_html), (exam.parse_question, exam_html)):
            with self.subTest(parser=parser.__module__):
                self.assertIs(parse(parser, factory(type_value="3", answer="false")).answer, False)
                blanks = parse(parser, factory(type_value="2"))
                self.assertEqual(blanks.answer, ["已有", ""])
                self.assertEqual(len(blanks.options), 2)

    def test_formula_structure_survives_question_and_option_parsing(self):
        fraction = "<math><mfrac><mi>a</mi><mi>b</mi></mfrac></math>"
        product = "<math><mrow><mi>a</mi><mi>b</mi></mrow></math>"
        tex = r'<script type="math/tex; mode=display">\frac{a}{b}</script>'
        cases = (
            ("x<sup>2</sup>+x<sub>i<sup>2</sup></sub>", "x<sup>2</sup>", "x2", "x^{2}+x_{i^{2}}"),
            (fraction, fraction, product, fraction),
            (tex, tex, r'<script type="math/tex">\frac{b}{a}</script>', r"\(\frac{a}{b}\)"),
        )
        for parser, factory in ((work.parse_question, work_html), (exam.parse_question, exam_html)):
            for body, first, second, expected in cases:
                with self.subTest(parser=parser.__module__, body=body):
                    node = BeautifulSoup(factory(), "lxml").div
                    targets = (
                        (node.select_one("div.Py-m1-title, div.tit"), body),
                        (
                            node.select_one(
                                "li.more-choose-item .choose-desc, div.answerList[name='A']"
                            ),
                            first,
                        ),
                        (
                            node.select_one(
                                "li.more-choose-item:nth-child(2) .choose-desc, div.answerList[name='B']"
                            ),
                            second,
                        ),
                    )
                    for target, html in targets:
                        target.clear()
                        for child in list(BeautifulSoup(html, "html.parser").contents):
                            target.append(child)
                    q = parser(node)
                    self.assertEqual(q.value, expected)
                    self.assertNotEqual(q.options["A"], q.options["B"])
                    self.assertEqual(resolver.normalize_answer(q, q.options["A"]), "A")
                    self.assertEqual(resolver.normalize_answer(q, q.options["B"]), "B")
                    if body != fraction:
                        self.assertIsNone(resolver.normalize_answer(q, "ab"))

    def test_inline_whitespace_separates_words_in_prompts_and_options(self):
        for parser, factory in ((work.parse_question, work_html), (exam.parse_question, exam_html)):
            with self.subTest(parser=parser.__module__):
                node = BeautifulSoup(factory(), "lxml").div
                for selector in (
                    "div.Py-m1-title, div.tit",
                    "li.more-choose-item .choose-desc, div.answerList[name='A']",
                ):
                    target = node.select_one(selector)
                    target.clear()
                    for child in list(
                        BeautifulSoup("<span>New</span>\n<span>York</span>", "html.parser").contents
                    ):
                        target.append(child)
                q = parser(node)
                self.assertEqual(q.value, "New York")
                self.assertEqual(q.options["A"], "New York")
                self.assertEqual(resolver.normalize_answer(q, "New York"), "A")
                self.assertIsNone(resolver.normalize_answer(q, "NewYork"))

    def test_exam_type_and_saved_answer_fields_use_exact_id_or_name(self):
        for raw, expected in (
            ("0", QuestionType.单选题),
            ("1", QuestionType.多选题),
            ("2", QuestionType.填空题),
            ("3", QuestionType.判断题),
        ):
            for identity in ("name", "id"):
                with self.subTest(raw=raw, identity=identity):
                    html = (
                        exam_html(type_value=raw, answer="false")
                        .replace(
                            '<input name="type42"',
                            f'<input name="type99" value="3"><input {identity}="type42"',
                        )
                        .replace('<input id="answer42"', '<input name="answer42"')
                    )
                    q = parse(exam.parse_question, html)
                    self.assertEqual(q.type, expected)
                    if raw == "3":
                        self.assertIs(q.answer, False)
        html = exam_html().replace(
            '<input name="type42"', '<input id="type42" value="1"><input name="type42"'
        )
        with self.assertRaises(ValueError):
            parse(exam.parse_question, html)

    def test_ambiguous_or_mismatched_question_identity_is_rejected(self):
        for parser, factory in ((work.parse_question, work_html), (exam.parse_question, exam_html)):
            with self.subTest(parser=parser.__module__):
                node = BeautifulSoup(factory(), "lxml").div
                for qid in ("42", "43"):
                    node.append(
                        BeautifulSoup(f'<input name="questionId" value="{qid}">', "html.parser")
                    )
                with self.assertRaises(ValueError):
                    parser(node)
        node = BeautifulSoup(work_html(), "lxml").div
        node.append(BeautifulSoup('<input name="questionId" value="43">', "html.parser"))
        with self.assertRaises(ValueError):
            work.parse_question(node)

    def test_saved_answer_collisions_are_rejected_and_identical_false_is_preserved(self):
        for parser, factory in ((work.parse_question, work_html), (exam.parse_question, exam_html)):
            for extra in ("false", "FALSE", "true", ""):
                with self.subTest(parser=parser.__module__, extra=extra):
                    node = BeautifulSoup(factory(type_value="3", answer="false"), "lxml").div
                    node.insert(
                        0,
                        BeautifulSoup(f'<input name="answer42" value="{extra}">', "html.parser"),
                    )
                    if extra.casefold() == "false":
                        self.assertIs(parser(node).answer, False)
                    else:
                        with self.assertRaises(ValueError):
                            parser(node)

    def test_single_choice_exact_unique_and_consistent(self):
        q = question()
        for raw, expected in (
            ("A", "A"),
            ("Ａ", "A"),
            ("A. 甲", "A"),
            ("甲", "A"),
            (["甲"], "A"),
            ("A. 乙", None),
            ("甲的解释", None),
            ("D", None),
        ):
            with self.subTest(raw=raw):
                self.assertEqual(resolver.normalize_answer(q, raw), expected)
        q.options["B"] = "甲"
        self.assertIsNone(resolver.normalize_answer(q, "甲"))
        self.assertEqual(resolver.normalize_answer(q, "A"), "A")

    def test_normalization_preserves_math_case_and_punctuation(self):
        q = question(options={"A": "x < 1", "B": "x ≤ 1", "C": "X < 1", "D": "x < 1!"})
        for raw, expected in (
            ("x &lt; 1", "A"),
            ("x ≤ 1", "B"),
            ("X < 1", "C"),
            ("x < 1!", "D"),
            ("x < 1.", None),
        ):
            with self.subTest(raw=raw):
                self.assertEqual(resolver.normalize_answer(q, raw), expected)
        self.assertNotEqual(normalize_text("不正确"), normalize_text("正确"))

    def test_multiple_choice_requires_every_part_and_option_order(self):
        q = question(QuestionType.多选题, {"C": "丙", "A": "甲", "B": "乙"})
        for raw, expected in (
            (["甲", "丙", "甲"], "CA"),
            ("A;C;A", "CA"),
            ("AC", "CA"),
            ("甲#丙", "CA"),
            (["甲", "不存在"], None),
            ("AD", None),
            ("", None),
            ([], None),
        ):
            with self.subTest(raw=raw):
                self.assertEqual(resolver.normalize_answer(q, raw), expected)

    def test_judgment_is_whole_token_and_bool_false_is_answer(self):
        q = question(QuestionType.判断题)
        for raw, expected in (
            (False, False),
            (True, True),
            ("FALSE", False),
            ("true", True),
            ("不正确", False),
            ("错误", False),
            ("正确", True),
            ("√", True),
            ("×", False),
            ("此说法正确因为...", None),
            ("不一定正确", None),
            (None, None),
            ("", None),
            (0, False),
            (1, True),
            (0.0, None),
            (1.0, None),
        ):
            with self.subTest(raw=raw):
                self.assertIs(resolver.normalize_answer(q, raw), expected)

    def test_blank_count_empty_and_existing_answer_conflict(self):
        q = question(QuestionType.填空题, ["一", "二"], ["已有", ""])
        for raw in (["已有"], ["已有", ""], ["已有", "新", "多余"], ["冲突", "新"]):
            with self.subTest(raw=raw):
                _, filled = self.resolve(q, [raw])
                self.assertFalse(filled)
                self.assertEqual(q.answer, ["已有", ""])
        _, filled = self.resolve(q, [["已有", "新"]])
        self.assertTrue(filled)
        self.assertEqual(q.answer, ["已有", "新"])
        self.assertEqual(
            resolver.normalize_answer(question(QuestionType.填空题, ["一", "二"]), "甲#乙"), ["甲", "乙"]
        )

    def test_partial_blanks_keep_original_existing_text(self):
        q = question(QuestionType.填空题, ["一", "二"], ["  已有  ", ""])
        _, filled = self.resolve(q, [["已有", "新"]])
        self.assertTrue(filled)
        self.assertEqual(q.answer, ["  已有  ", "新"])

    def test_equivalent_sources_merge_and_conflicts_do_not_fill(self):
        q = question()
        _, filled = self.resolve(q, ["A", "甲", "Ａ. 甲"])
        self.assertTrue(filled)
        self.assertEqual(q.answer, "A")
        q = question()
        _, filled = self.resolve(q, ["A", "B"])
        self.assertFalse(filled)
        self.assertIsNone(q.answer)
        q = question(QuestionType.判断题)
        _, filled = self.resolve(q, [False, "FALSE", "错误"])
        self.assertTrue(filled)
        self.assertIs(q.answer, False)

    def test_explicit_source_conflict_blocks_other_valid_source(self):
        q = question()
        instance = resolver.QuestionResolver(FakeDto([q]))
        self.assertFalse(instance.fill(q, [response("A"), response(None, code=-409)]))
        self.assertIsNone(q.answer)

    def test_searcher_failure_does_not_hide_other_sources(self):
        first = SearcherBase()
        first.invoke = MagicMock(side_effect=RuntimeError("failure"))
        second = SearcherBase()
        second.invoke = MagicMock(return_value=response("A"))
        searchers = MultiSearcherWraper()
        searchers.add(first)
        searchers.add(second)
        results = searchers.invoke(question())
        self.assertEqual(len(results), 2)
        self.assertNotEqual(results[0].code, 0)
        self.assertEqual(results[1].answer, "A")

    def test_explicit_answer_field_or_separate_analysis_only(self):
        q = question()
        for raw in ('{"answer": "A"}', "答案：A\n解析：说明"):
            with self.subTest(raw=raw):
                self.assertEqual(resolver.normalize_answer(q, raw), "A")
        for raw in ("分析 A 正确，因此选择 A", "答案：A 因为甲符合条件", '{"explanation": "A"}'):
            with self.subTest(raw=raw):
                self.assertIsNone(resolver.normalize_answer(q, raw))

    def test_json_objects_and_full_fences_keep_typed_answers(self):
        q = question()
        for raw in (
            {"answer": "A"},
            {"answers": ["A"]},
            '```json\n{"answer":"A"}\n```',
            "```text\n答案：A\n```",
            '```\n"A"\n```',
        ):
            with self.subTest(raw=raw):
                self.assertEqual(resolver.normalize_answer(q, raw), "A")
        judgment = question(QuestionType.判断题)
        for raw in ({"answer": False}, '{"answers":false}', '```json\n{"answer":false}\n```'):
            with self.subTest(raw=raw):
                self.assertIs(resolver.normalize_answer(judgment, raw), False)
        blanks = question(QuestionType.填空题, ["一", "二"])
        self.assertEqual(resolver.normalize_answer(blanks, {"answers": ["甲", "乙"]}), ["甲", "乙"])
        multiple = question(QuestionType.多选题)
        self.assertEqual(resolver.normalize_answer(multiple, {"answers": ["A", "B"]}), "AB")

    def test_thinking_is_removed_only_when_complete_and_leading(self):
        q = question()
        for raw in ("<think>推理</think>\n答案：A", '<THINK>推理</THINK>\n```json\n{"answer":"A"}\n```'):
            with self.subTest(raw=raw):
                self.assertEqual(resolver.normalize_answer(q, raw), "A")
        blanks = question(QuestionType.填空题, ["一"])
        for raw in (
            "<think>未结束",
            "<think >未结束",
            "答案：A<think>推理</think>",
            "<think><think>推理</think>A",
            ["<think>未结束"],
            {"answer": "<think>推理</think>A"},
        ):
            with self.subTest(raw=raw):
                self.assertIsNone(resolver.normalize_answer(q, raw))
                self.assertIsNone(resolver.normalize_answer(blanks, raw))
                dto = FakeDto([blanks])
                instance, _ = self.execute(dto, [raw], fallback_save=False)
                self.assertEqual(instance.completed_cnt, 0)
                dto.submit.assert_not_called()
                dto.final_submit.assert_not_called()

    def test_malformed_wrappers_and_duplicate_fields_cannot_choose_an_answer(self):
        q = question()
        for raw in (
            '{"answer":"A","answer":"B"}',
            '{"answer":"A","answers":"B"}',
            {"answer": "A", "答案": "B"},
            '```json\n{"answer":"A"}',
            '```json\n{"answer":"A"}\n``` trailing',
            'prefix ```json\n{"answer":"A"}\n```',
            '{"answer":"A"} trailing',
        ):
            with self.subTest(raw=raw):
                self.assertIsNone(resolver.normalize_answer(q, raw))

    def test_same_line_analysis_is_not_a_blank_answer(self):
        q = question(QuestionType.填空题, ["一"])
        for raw in (
            "答案：北京，解析：北京是首都",
            ["北京，解析：北京是首都"],
            '["北京，解析：北京是首都"]',
            '{"answer":["北京，解析：北京是首都"]}',
            ["北京，解\u200b析：北京是首都"],
        ):
            with self.subTest(raw=raw):
                self.assertIsNone(resolver.normalize_answer(q, raw))
                dto = FakeDto([q])
                instance, _ = self.execute(dto, [raw], fallback_save=False)
                self.assertEqual(instance.completed_cnt, 0)
                dto.submit.assert_not_called()
                dto.final_submit.assert_not_called()
        self.assertEqual(resolver.normalize_answer(q, "答案：北京\n解析：北京是首都"), ["北京"])

    def test_numeric_interval_text_is_not_unwrapped_as_json_array(self):
        q = question(options={"A": "[0,1]", "B": "(0,1)"})
        self.assertEqual(resolver.normalize_answer(q, "[0,1]"), "A")
        self.assertEqual(resolver.normalize_answer(q, "(0,1)"), "B")

    def test_fuzzer_setting_never_invents_answer(self):
        q = question()
        _, filled = self.resolve(q, [], fallback_fuzzer=True)
        self.assertFalse(filled)
        self.assertIsNone(q.answer)

    def test_json_exact_question_lookup_preserves_negation_and_false(self):
        source = self.folder / "bank.json"
        source.write_text(
            json.dumps({"以下不正确的是？": "B", "以下正确的是？": "A", "判断\u00a0题": False}, ensure_ascii=False),
            encoding="utf8",
        )
        searcher = JsonFileSearcher(source)
        self.assertEqual(searcher.invoke(question(value="以下正确的是？")).answer, "A")
        self.assertNotEqual(searcher.invoke(question(value="以下正确的是")).code, 0)
        self.assertIs(searcher.invoke(question(QuestionType.判断题, value="判断 题")).answer, False)

    def test_json_normalized_duplicate_answers_must_agree(self):
        source = self.folder / "bank.json"
        source.write_text('{"测试题": "A", " 测试题 ": "甲"}', encoding="utf8")
        self.assertEqual(JsonFileSearcher(source).invoke(question()).code, 0)
        source.write_text('{"测试题": "A", "测试题": "B"}', encoding="utf8")
        self.assertNotEqual(JsonFileSearcher(source).invoke(question()).code, 0)

    def test_json_searcher_wrappers_preserve_false_and_reject_duplicate_answer_fields(self):
        source = self.folder / "bank.json"
        source.write_text('{"测试题":{"answer":false}}', encoding="utf8")
        q = question(QuestionType.判断题)
        result = JsonFileSearcher(source).invoke(q)
        self.assertIs(resolver.normalize_answer(q, result.answer), False)
        source.write_text('{"测试题":{"answers":["甲","乙"]}}', encoding="utf8")
        q = question(QuestionType.填空题, ["一", "二"])
        self.assertEqual(
            resolver.normalize_answer(q, JsonFileSearcher(source).invoke(q).answer), ["甲", "乙"]
        )
        source.write_text('{"测试题":{"answer":"A","answer":"B"}}', encoding="utf8")
        q = question()
        self.assertIsNone(resolver.normalize_answer(q, JsonFileSearcher(source).invoke(q).answer))

    def test_existing_valid_answers_skip_search_and_single_submit(self):
        for q in (
            question(answer="A"),
            question(QuestionType.判断题, answer=False),
            question(QuestionType.填空题, ["一", "二"], ["甲", "乙"]),
        ):
            with self.subTest(kind=q.type):
                dto = FakeDto([q])
                instance, searcher = self.execute(dto)
                self.assertEqual((instance.completed_cnt, instance.incompleted_cnt), (1, 0))
                searcher.invoke.assert_not_called()
                dto.submit.assert_not_called()
                dto.final_submit.assert_called_once()

    def test_constructor_does_not_require_search_configuration(self):
        with patch.object(resolver, "load_searcher", side_effect=AssertionError("eager loading")):
            resolver.QuestionResolver(FakeDto([]))

    def test_failed_receipt_or_exception_never_counts_as_completed(self):
        for receipt in ({"status": False}, {"status": "error"}, {}, {"status": "false"}):
            with self.subTest(receipt=receipt):
                dto = FakeDto([question()], receipt)
                instance, _ = self.execute(dto, fallback_save=False)
                self.assertEqual((instance.completed_cnt, instance.incompleted_cnt), (0, 1))
                dto.final_submit.assert_not_called()
        dto = FakeDto([question()])
        dto.submit.side_effect = APIError("offline failure")
        instance, _ = self.execute(dto, fallback_save=False)
        self.assertEqual((instance.completed_cnt, instance.incompleted_cnt), (0, 1))
        dto.final_submit.assert_not_called()

    def test_unmatched_unknown_and_parse_errors_block_final_submit(self):
        for questions, errors, answers in (
            ([question()], {}, ("不存在",)),
            ([question(QuestionType.其它)], {}, ("A",)),
            ([question(answer="A")], {2: "missing ID"}, ()),
            ([question(answer="A"), question(answer="B")], {}, ()),
        ):
            with self.subTest(errors=errors, kind=questions[0].type):
                dto = FakeDto(questions, parse_errors=errors)
                instance, searcher = self.execute(dto, answers, fallback_save=False)
                self.assertGreater(instance.incompleted_cnt, 0)
                dto.final_submit.assert_not_called()
                dto.submit.assert_not_called()
                if len(questions) == 2:
                    self.assertEqual((instance.completed_cnt, instance.incompleted_cnt), (1, 1))
                    searcher.invoke.assert_not_called()

    def test_work_local_cache_receipt_counts_without_claiming_network(self):
        q = question(answer="A")
        dto = self.work_dto(work_html(answer="A"))
        q = dto.fetch_all()[0]
        receipt = dto.submit(index=0, question=q)
        dto.session.post.assert_not_called()
        fake = FakeDto([question(value=q.value)], receipt)
        instance, _ = self.execute(fake, auto_final_submit=False)
        self.assertEqual((instance.completed_cnt, instance.incompleted_cnt), (1, 0))
        fake.final_submit.assert_not_called()

    def test_work_form_omits_invalid_answers_without_false_coercion(self):
        invalid = [
            question(QuestionType.判断题, qid=1),
            question(QuestionType.判断题, answer="false", qid=2),
            question(QuestionType.填空题, ["一", "二"], ["甲"], qid=3),
            question(answer="Z", qid=4),
        ]
        form = work.construct_questions_form(
            invalid + [question(QuestionType.判断题, answer=False, qid=5)]
        )
        self.assertEqual(form["answer5"], "false")
        for key in ("answer1", "answer2", "answer31", "tiankongsize3", "answer4"):
            self.assertNotIn(key, form)

    def test_exam_form_omits_invalid_answers_and_encodes_false(self):
        for q in (
            question(QuestionType.判断题),
            question(QuestionType.判断题, answer="false"),
            question(QuestionType.填空题, ["一", "二"], ["甲"]),
        ):
            with self.subTest(q=q):
                form = exam.construct_question_form(q)
                self.assertFalse(any(key.startswith(("answer", "blankNum")) for key in form))
        self.assertEqual(
            exam.construct_question_form(question(QuestionType.判断题, answer=False))["answer42"],
            "false",
        )

    def test_exam_invalid_answers_are_rejected_before_network(self):
        dto = self.exam_dto("")
        for q in (
            question(QuestionType.判断题),
            question(QuestionType.判断题, answer="false"),
            question(answer="Z"),
        ):
            with self.subTest(q=q), self.assertRaises(APIError):
                dto.submit(question=q)
        dto.session.post.assert_not_called()

    def test_failed_final_receipt_is_not_reported_as_success(self):
        dto = FakeDto([question(answer="A")])
        dto.final_submit.return_value = {"status": False}
        instance, _ = self.execute(dto)
        messages = [str(call.args[0]) for call in instance.logger.info.call_args_list]
        self.assertFalse(any("交卷成功" in message for message in messages))

    def test_models_return_raw_output_and_keep_negative_question(self):
        raw = "答案：B\n解析：不正确的是乙"
        q = question(value="以下不正确的是？")
        settings = dict(
            api_key="test",
            base_url="https://invalid.test",
            model="test",
            prompt="{type}：{value}\n{options}",
            system_prompt="回答题目",
        )
        for cls in (OpenAISearcher, OllamaSearcherAPI):
            with self.subTest(searcher=cls.__name__):
                instance = cls.__new__(cls)
                instance.config = settings
                instance.logger = MagicMock()
                if cls is OpenAISearcher:
                    instance.client = MagicMock()
                    instance.client.chat.completions.create.return_value = SimpleNamespace(
                        choices=[SimpleNamespace(message=SimpleNamespace(content=raw))]
                    )
                    result = instance.invoke(q)
                    sent = instance.client.chat.completions.create.call_args.kwargs["messages"][1][
                        "content"
                    ]
                else:
                    payload = {"response": raw}
                    with patch(f"{cls.__module__}.requests.post") as post:
                        post.return_value.json.return_value = payload
                        result = instance.invoke(q)
                        sent = json.dumps(post.call_args.kwargs["json"], ensure_ascii=False)
                self.assertEqual(result.answer, raw)
                self.assertIn("以下不正确的是？", sent)


if __name__ == "__main__":
    unittest.main()
