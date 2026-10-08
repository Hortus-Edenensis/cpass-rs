import logging
import re
from pathlib import Path
from typing import Literal

from bs4 import BeautifulSoup, Tag

from logger import Logger

from ..base import QAQDtoBase, TaskPointBase
from ..exception import PointWorkError, WorkAccessDenied
from ..schema import (
    SUPPORTED_QUESTION_TYPES,
    QuestionModel,
    QuestionsExportSchema,
    QuestionsExportType,
    QuestionType,
    is_valid_answer,
    parse_question_type,
)
from ..utils import html_text, question_text, question_type_label, saved_answer_value

# 接口-单元作业答题提交
API_WORK_COMMIT = "https://mooc1-api.chaoxing.com/work/addStudentWorkNew"

# SSR页面-客户端单元测验答题页
PAGE_MOBILE_WORK = "https://mooc1-api.chaoxing.com/android/mworkspecial"


def parse_question(question_node: Tag) -> QuestionModel:
    """解析题目
    Args:
        question_node: 题目 html 标签节点
    Returns:
        QuestionModel: 题目数据模型
    """
    if question_node is None:
        raise ValueError("缺少题目节点")
    type_nodes = question_node.find_all("input", id=re.compile(r"^answertype[0-9]+$"))
    if len(type_nodes) > 1:
        raise ValueError("题目 ID 不唯一")
    id_nodes = question_node.select("input[name='questionId']")
    if len(id_nodes) > 1:
        raise ValueError("题目 ID 不唯一")
    id_node = id_nodes[0] if id_nodes else None
    raw_id = (
        id_node.get("value")
        if id_node
        else type_nodes[0]["id"][10:]
        if type_nodes
        else question_node.get("data")
    )
    if not raw_id or not re.fullmatch(r"[0-9]+", raw_id) or int(raw_id) <= 0:
        raise ValueError("缺少可信题目 ID")
    question_id = int(raw_id)
    if type_nodes and int(type_nodes[0]["id"][10:]) != question_id:
        raise ValueError("题目 ID 与题型字段不一致")
    type_node = type_nodes[0] if type_nodes else None
    title_node = question_node.select_one("div.Py-m1-title")
    raw_type = type_node.get("value") if type_node else None
    type_title = html_text(question_type_label(title_node) or title_node)
    question_type = parse_question_type(raw_type, type_title)
    if question_type not in SUPPORTED_QUESTION_TYPES:
        logging.getLogger("PointWork").warning(
            "题型未识别或未支持: id=%s type=%r title=%r", question_id, raw_type, type_title
        )
    question_value = question_text(title_node)
    options = None
    answer = None
    saved_answer = saved_answer_value(
        question_node,
        f"input.answerInput, input[id='answer{question_id}'], input[id='answers{question_id}'], "
        f"input[name='answer{question_id}'], input[name='answers{question_id}']",
    )
    match question_type:
        case QuestionType.单选题 | QuestionType.多选题:
            options = {}
            answer = saved_answer or None
            for option_node in question_node.select("li.more-choose-item"):
                key_node = option_node.select_one("em.choose-opt")
                option_key = key_node.get("id-param") if key_node else None
                if not option_key or option_key in options:
                    raise ValueError("选项标号缺失或重复")
                body = option_node.select_one("div.choose-desc")
                if body is None:
                    raise ValueError("缺少选项正文")
                option_value = html_text(body.select_one("cc") or body)
                option_value = re.sub(
                    rf"^\s*{re.escape(option_key)}\s*[.．、:：)）]\s*", "", option_value
                )
                if not option_value:
                    raise ValueError("选项正文为空")
                options[option_key] = option_value
            if not options:
                raise ValueError("缺少选项")
        case QuestionType.填空题:
            options = []
            answer = []
            for blank_node in question_node.select("ul.blankList2 > li"):
                label = blank_node.select_one("span")
                field = blank_node.select_one("input.blankInp2")
                if field is None:
                    raise ValueError("缺少填空输入框")
                options.append(html_text(label) if label else "")
                answer.append(field.get("value", ""))
            if not options:
                raise ValueError("缺少填空项")
        case QuestionType.判断题:
            answer = {"true": True, "false": False}.get(saved_answer.strip().lower())

    return QuestionModel(
        id=question_id,
        value=question_value,
        type=question_type,
        options=options,
        answer=answer,
    )


def construct_questions_form(questions: list[QuestionModel]) -> dict[str, int | str]:
    """构建答题表单
    Args:
        list[question]: 需要构建的题单, 成员为题目数据模型
    Retuerns:
        dict: 用于提交的题目表单
    不同类型的题目会生成 key-value 不同的表单
    会生成包含多道题目信息的表单
    """
    questions = [question for question in questions if is_valid_answer(question)]
    form = {"answerwqbid": ",".join(str(q.id) for q in questions)}
    for question in questions:
        form[f"answertype{question.id}"] = question.type.value
        match question.type:
            case QuestionType.单选题 | QuestionType.多选题:
                form[f"answer{question.id}"] = question.answer
            case QuestionType.填空题:
                blank_amount = len(question.options)
                form[f"tiankongsize{question.id}"] = blank_amount
                for blank_index in range(blank_amount):
                    form[f"answer{question.id}{blank_index + 1}"] = question.answer[blank_index]
            case QuestionType.判断题:
                form[f"answer{question.id}"] = "true" if question.answer else "false"
            case _:
                raise NotImplementedError
    return form


class PointWorkDto(TaskPointBase, QAQDtoBase):
    """作业任务点接口 (手机客户端协议)"""

    title: str
    work_id: str
    school_id: str
    job_id: str
    ktoken: str
    enc: str
    # 提交参数
    work_answer_id: int
    total_question_num: str
    full_score: str
    work_relation_id: int
    enc_work: str
    need_jobid: bool  # 是否需要提供 job_id

    # 当前作业的全部题目
    questions: list[QuestionModel]

    def __init__(self, work_id: str, school_id: str, job_id: str, **kwargs) -> None:
        super(PointWorkDto, self).__init__(**kwargs)
        super(TaskPointBase, self).__init__()
        self.logger = Logger("PointWork")

        self.work_id = work_id
        self.school_id = school_id
        self.job_id = job_id

        self.questions = []
        self._question_indices: list[int] = []

    def parse_attachment(self) -> bool:
        """解析任务点卡片 Attachment
        Returns:
            bool: 是否需要完成
        """
        try:
            # 定位资源 workid
            for point in self.attachment["attachments"]:
                if prop := point.get("property"):
                    if prop.get("workid") == self.work_id:
                        break
            else:
                self.logger.warning("定位任务资源失败")
                return False
            self.ktoken = self.attachment["defaults"]["ktoken"]
            self.enc = point["enc"]
            job = point.get("job")
            needtodo = job is True
            self.logger.info("解析Attachment成功")
        except Exception:
            self.logger.error("解析Attachment失败")
            raise RuntimeError("解析试题Attachment出错")
        return needtodo

    def __str__(self) -> str:
        return f"PointWork(title={self.title} jobid={self.job_id} workid={self.work_id})"

    def __next__(self) -> tuple[int, QuestionModel]:
        """迭代返回
        Returns:
            int, QuestionModel: 题目索引 题目模型
        """
        if self.current_index >= len(self.questions):
            raise StopIteration
        question = self.questions[self.current_index]
        index = self._question_indices[self.current_index]
        self.current_index += 1
        return index, question

    def fetch(self, index: int) -> QuestionModel:
        """拉取一道题
        Args:
            index: 题目索引 从 0 开始计数
        Returns:
            QuestionModel: 题目模型
        """

        # 如没有缓存题单, 则立即缓存
        if not self.questions:
            self.fetch_all()

        return self.questions[self._question_indices.index(index)]

    def fetch_all(self) -> list[QuestionModel]:
        """拉取全部作业题单
        Returns:
            list[QuestionModel]: 题目模型列表 (按题号顺序排列)
        """
        resp = self.session.get(
            PAGE_MOBILE_WORK,
            params={
                "courseid": self.course_id,
                "workid": f"{self.school_id}-{self.work_id}"
                if self.school_id
                else self.work_id,  # 这里分类讨论两种形式的 workid
                # "jobid": self.job_id if self.need_jobid else "",
                "jobid": self.job_id,
                "needRedirect": "true",
                "knowledgeid": self.knowledge_id,
                "userid": self.session.acc.puid,
                "ut": "s",
                "clazzId": self.class_id,
                "cpi": self.cpi,
                "ktoken": self.ktoken,
                "enc": self.enc,
            },
        )
        resp.raise_for_status()
        html = BeautifulSoup(resp.text, "lxml")

        # 解析错误提示信息
        if p := html.select_one("p.blankTips"):
            self.logger.warning(f"作业拉取失败 ({p.text}) [J.{self.job_id}/W.{self.work_id}]")
            if p.text == "无效的权限":
                raise WorkAccessDenied
            elif p.text == "此作业已被老师删除！":
                raise WorkAccessDenied
            else:
                raise PointWorkError(p.text)

        # 已批阅解析逻辑
        if re.search(r"已批阅", html.head.title.text):
            self.logger.warning("作业已批阅")
            raise NotImplementedError("暂不支持解析已批阅作业")

        # 解析公共参数
        submit_form = html.body.select_one("form#form1")
        if submit_form is None:
            raise PointWorkError("作业未创建完成")

        self.title = html.body.select_one("h3.py-Title,h3.chapter-title").text.strip()
        self.work_answer_id = int(submit_form.select_one("input#workAnswerId")["value"])
        total_node = submit_form.select_one("input#totalQuestionNum")
        self.total_question_num = total_node.get("value", "") if total_node else ""
        self.work_relation_id = int(submit_form.select_one("input#workRelationId")["value"])
        self.full_score = submit_form.select_one("input#fullScore")["value"]
        self.enc_work = submit_form.select_one("input#enc_work")["value"]
        self.logger.info(f"作业拉取成功 [{self.title}(J.{self.job_id}/W.{self.work_id})]")

        # 解析题目数据
        question_nodes = html.body.select("div.Py-mian1")
        self.questions = []
        self._question_indices = []
        self.parse_errors.clear()
        expected = (
            int(self.total_question_num)
            if re.fullmatch(r"[0-9]+", str(self.total_question_num))
            else 0
        )
        if expected <= 0:
            self.parse_errors[0] = f"题目总数无效: {self.total_question_num!r}"
        elif len(question_nodes) < expected:
            for index in range(len(question_nodes), expected):
                self.parse_errors[index] = "缺少题目节点"
        elif len(question_nodes) > expected:
            for index in range(expected, len(question_nodes)):
                self.parse_errors[index] = "题目节点超出声明总数"
        for index, error in self.parse_errors.items():
            self.logger.warning(f"题目 {index} 解析失败: {error}")
        seen_ids = set()
        for index, question_node in enumerate(question_nodes):
            try:
                question = parse_question(question_node)
                if question.id in seen_ids:
                    raise ValueError(f"题目 ID 重复: {question.id}")
            except (ValueError, TypeError, KeyError, AttributeError) as exc:
                self.parse_errors[index] = str(exc)
                self.logger.warning(f"题目 {index} 解析失败: {exc}")
                continue
            seen_ids.add(question.id)
            self._question_indices.append(index)
            self.questions.append(question)
        self.logger.info(f"作业题单解析成功 [{self.title}(J.{self.job_id}/W.{self.work_id})]")
        self.logger.info(f"已缓存共 {len(self.questions)} 道题")
        self.logger.debug(f"题目 list: {[question.to_dict() for question in self.questions]}")
        return self.questions

    def submit(
        self,
        *,
        index: int = 0,
        question: QuestionModel,
    ) -> dict:
        """提交一道题的答案
        Args:
            index: 题目索引
            question: 题目数据模型
        """
        if not is_valid_answer(question):
            raise PointWorkError("答案无效或未完成")
        cached_index = self._question_indices.index(index)
        if self.questions[cached_index].id != question.id:
            raise PointWorkError("题目 ID 与缓存不一致")
        self.questions[cached_index] = question
        self.logger.info(f"已提交题目 ({index}) 至缓存区 {question}")

        return {
            "status": True,
            "scope": "cache",
            "index": index,
            "question": question.value,
            "answer": question.answer,
        }

    def final_submit(self) -> dict:
        """提交所有题目并交卷
        Returns:
            dict: json 响应数据
        """
        expected = (
            int(self.total_question_num)
            if re.fullmatch(r"[0-9]+", str(self.total_question_num))
            else 0
        )
        if (
            expected <= 0
            or self.parse_errors
            or len(self.questions) != expected
            or len({question.id for question in self.questions}) != expected
            or not all(is_valid_answer(question) for question in self.questions)
        ):
            raise PointWorkError("存在未完成或解析失败题目，不能交卷")
        resp = self.session.post(
            API_WORK_COMMIT,
            params={
                "keyboardDisplayRequiresUserAction": 1,
                "_classId": self.class_id,
                "courseid": self.course_id,
                "token": self.enc_work,
                "workAnswerId": self.work_answer_id,
                "workid": self.work_relation_id,
                "cpi:": self.cpi,
                "jobid": self.job_id,
                "knowledgeid": self.knowledge_id,
                "ua": "app",
            },
            data={
                "pyFlag": "",
                "courseId": self.course_id,
                "classId": self.class_id,
                "api": 1,
                "mooc": 0,
                "workAnswerId": self.work_answer_id,
                "totalQuestionNum": self.total_question_num,
                "fullScore": self.full_score,
                "knowledgeid": self.knowledge_id,
                "oldSchoolId": "",
                "oldWorkId": self.work_id,
                "jobid": self.job_id,
                "workRelationId": self.work_relation_id,
                "enc_work": self.enc_work,
                "isphone": "true",
                "userId": self.session.acc.puid,
                "workTimesEnc": "",
                # 构建答题提交表单
                **construct_questions_form(self.questions),
            },
        )
        resp.raise_for_status()
        json_content = resp.json()
        self.logger.debug(f"试题提交 Resp: {json_content}")

        # 解析失败参数
        if (json_content.get("status")) != True:
            self.logger.error(
                f"交卷失败 ({json_content.get('msg')}) [{self.title}(J.{self.job_id}/W.{self.work_id})]"
            )
            raise PointWorkError(json_content.get("msg"))

        self.logger.info(
            f"交卷成功 ({json_content.get('msg')}) [{self.title}(J.{self.job_id}/W.{self.work_id})]"
        )
        return json_content

    def fallback_save(self) -> dict:
        """保存答题信息
        Returns:
            dict: json 响应数据
        """
        resp = self.session.post(
            API_WORK_COMMIT,
            params={
                "_classId": self.class_id,
                "courseid": self.course_id,
                "token": self.enc_work,
                "workAnswerId": self.work_answer_id,
                "ua": "app",
                "formType2": "post",
                "saveStatus": 1,
                "version": 1,
                "tempsave": 1,
            },
            data={
                "pyFlag": "1",
                "courseId": self.course_id,
                "classId": self.class_id,
                "api": 1,
                "mooc": 0,
                "workAnswerId": self.work_answer_id,
                "totalQuestionNum": self.total_question_num,
                "fullScore": self.full_score,
                "knowledgeid": self.knowledge_id,
                "oldSchoolId": "",
                "oldWorkId": self.work_id,
                "jobid": self.job_id,
                "workRelationId": self.work_relation_id,
                "enc_work": self.enc_work,
                "isphone": "true",
                "userId": self.session.acc.puid,
                "workTimesEnc": "",
                # 构建答题提交表单
                **construct_questions_form(self.questions),
            },
        )
        resp.raise_for_status()
        json_content = resp.json()
        self.logger.debug(f"试题保存 Resp: {json_content}")

        # 解析失败参数
        if (json_content.get("status")) != True:
            self.logger.error(
                f"保存失败 ({json_content.get('msg')}) [{self.title}(J.{self.job_id}/W.{self.work_id})]"
            )
            raise PointWorkError(json_content.get("msg"))

        self.logger.info(
            f"保存成功 ({json_content.get('msg')}) [{self.title}(J.{self.job_id}/W.{self.work_id})]"
        )
        return json_content

    def export(
        self,
        format_or_path: Literal["schema", "dict", "json"] | Path = "schema",
    ) -> QuestionsExportSchema | str | dict | None:
        """导出当前试题
        Args:
            format_or_path: 导出格式或路径
        """
        if not self.questions:
            self.fetch_all()

        schema = QuestionsExportSchema(
            id=self.work_id,
            title=self.title,
            type=QuestionsExportType.Work,
            questions=self.questions,
        )
        self.logger.info(f"导出全部试题 ({format}) [{self.title}(J.{self.job_id}/W.{self.work_id})]")
        if isinstance(format_or_path, Path):
            # 按路径导出
            with format_or_path.open("w", encoding="utf8") as fp:
                fp.write(schema.to_json(ensure_ascii=False, separators=(",", ":")))
        else:
            match format_or_path:
                case "schema":
                    return schema
                case "dict":
                    return schema.to_dict()
                case "json":
                    return schema.to_json(ensure_ascii=False, separators=(",", ":"))
                case _:
                    raise TypeError("未定义的导出类型")


__all__ = ["PointWorkDto"]
