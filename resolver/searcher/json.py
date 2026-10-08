import json
from pathlib import Path

from cxapi.schema import QuestionModel
from cxapi.utils import normalize_text

from . import SearcherBase, SearcherResp


class JsonFileSearcher(SearcherBase):
    "JSON 数据库搜索器"
    db: dict[str, str | list[str] | bool | int | None]

    def __init__(self, file_path: Path | str) -> None:
        try:
            with open(file_path, "r", encoding="utf8") as fp:
                # Preserve duplicate keys so contradictory entries cannot silently overwrite.
                self._entries = json.load(fp, object_pairs_hook=tuple)
            if not isinstance(self._entries, tuple):
                raise RuntimeError("JSON 题库必须是题目到答案的对象")
            self.db = dict(self._entries)
        except FileNotFoundError:
            raise RuntimeError("JSON 题库文件无效, 请检查配置")

    def invoke(self, question: QuestionModel) -> SearcherResp:
        value = normalize_text(question.value)
        matches = [(q, a) for q, a in self._entries if normalize_text(q) == value]
        if not matches:
            return SearcherResp(-404, "题目未匹配", self, question.value, None)
        if len(matches) > 1:
            from resolver.question import normalize_answer

            valid = [
                (q, a, normalized)
                for q, a in matches
                if (normalized := normalize_answer(question, a)) is not None
            ]
            if not valid:
                return SearcherResp(-404, "题目未匹配", self, question.value, None)
            if any(normalized != valid[0][2] for _, _, normalized in valid[1:]):
                return SearcherResp(-409, "题库答案冲突", self, question.value, None)
            q, answer, _ = valid[0]
            return SearcherResp(0, "ok", self, q, answer)
        q, answer = matches[0]
        return SearcherResp(0, "ok", self, q, answer)


__all__ = ["JsonFileSearcher"]
