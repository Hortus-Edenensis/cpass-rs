import hashlib
import random
import re
import secrets
import time
import urllib.parse
from copy import copy
from html import unescape
from math import floor
from typing import Literal

from bs4.element import Comment

# API 环境参数
IMEI = secrets.token_hex(16)  # 设备uuid 生成随机即可
ANDROID_VERSION = f"Android {random.randint(9, 12)}"  # 安卓版本
MODEL = f"MI{random.randint(10, 12)}"  # 设备型号
LOCALE = "zh_CN"  # 语言标识
VERSION = "6.3.9"  # 版本名
BUILD = "10824_250"  # 构建id


def inf_enc_sign(params: dict) -> dict:
    """为请求表单添加 infenc 签名
    Args:
        params: 原始请求表单参数
    Returns:
        dict: 加入签名后的表单参数
    """
    query = urllib.parse.urlencode(params) + "&DESKey=Z(AfY@XS"
    inf_enc = hashlib.md5(query.encode()).hexdigest()
    return {
        **params,
        "inf_enc": inf_enc,
    }


def get_ts() -> str:
    """获取字符串形式当前时间戳
    Returns:
        str: 时间戳
    """
    return f"{round(time.time() * 1000)}"


def get_imei() -> str:
    """获取 IMEI
    Returns:
        str: 虚拟IMEI
    """
    return IMEI


def mobile_ua_sign(model: str, locale: str, version: str, build: str, imei: str) -> str:
    """客户端 UA 签名
    Args:
        model: 设备型号
        locale: 语言标识
        version: 版本名
        build: 构建id
        imei: 设备uuid
    Returns:
        str: schild字段签名值
    """
    return hashlib.md5(
        " ".join(
            (
                f"(schild:ipL$TkeiEmfy1gTXb2XHrdLN0a@7c^vu)",
                f"(device:{model})",
                f"Language/{locale}",
                f"com.chaoxing.mobile/ChaoXingStudy_3_{version}_android_phone_{build}",
                f"(@Kalimdor)_{imei}",
            )
        ).encode()
    ).hexdigest()


def get_ua(ua_type: Literal["mobile", "web"]) -> str:
    """获取 UA
    Args:
        ua_type: UA类型
    Returns:
        str: UA字串
    """
    match ua_type:
        case "mobile":
            return " ".join(
                (
                    f"Dalvik/2.1.0 (Linux; U; {ANDROID_VERSION}; {MODEL} Build/SKQ1.211006.001)",
                    f"(schild:{mobile_ua_sign(MODEL, LOCALE,VERSION,BUILD,IMEI)})",
                    f"(device:{MODEL})",
                    f"Language/{LOCALE}",
                    f"com.chaoxing.mobile/ChaoXingStudy_3_{VERSION}_android_phone_{BUILD}",
                    f"(@Kalimdor)_{IMEI}",
                )
            )
        case "web":
            return "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/107.0.0.0 Safari/537.36 Edg/107.0.1418.35"
        case _:
            raise NotImplementedError


def get_exam_signature(uid: int, qid: int, x: int, y: int):
    """计算考试提交接口签名参数组
    Args:
        uid: 用户 puid
        qid: 题目 id (可为 0)
        x: 屏幕点击坐标 X
        y: 屏幕点击坐标 Y
    Returns:
        dict: 签名参数组 pos rd value _edt
    """
    ts = get_ts()
    r1 = random.randrange(0, 9)
    r2 = random.randrange(0, 9)
    a = f"{secrets.token_hex(16)}{ts[4:]}{r1}{r2}{qid or ''}"
    temp = 0
    for ch in a:
        temp = (temp << 5) - temp + ord(ch)
    salt = f"{r1}{r2}{(0x7fffffff & temp) % 10}"
    encVal = f"{uid}"
    if qid:
        encVal += f"_{qid}"
    encVal += f"|{salt}"
    encVal2 = "".join(str(ord(c)) for c in encVal)
    b = len(encVal2) // 5
    c = int(encVal2[b] + encVal2[2 * b] + encVal2[3 * b] + encVal2[4 * b])
    d = len(encVal) // 2 + 1
    e = (c * int(encVal2[:10]) + d) % 0x7FFFFFFF
    pos = f"({x}|{y})"
    result = ""
    for ch in pos:
        temp = ord(ch) ^ floor(e / 0x7FFFFFFF * 0xFF)
        result += f"{temp:02x}"
        e = (c * e + d) % 0x7FFFFFFF

    return {
        "pos": f"{result}{secrets.token_hex(4)}",
        "rd": random.random(),
        "value": pos,
        "_edt": f"{ts}{salt}",
    }


def remove_escape_chars(text: str) -> str:
    """移除空白字符
    Args:
        text: 输入字符串
    Returns:
        str: 输出字符串
    """
    return (
        text.replace("\xa0", " ")
        .strip()
        .replace("\u2002", "")
        .replace("\u200b", "")
        .replace("\u3000", "")
    )


def normalize_text(text: str) -> str:
    """清理实体和排版字符，保留大小写、公式及标点。"""
    return " ".join(re.sub(r"[\u200b\u200c\u200d\u2060\ufeff]", "", unescape(text)).split())


def html_text(node) -> str:
    """保留行内文字连接以及显式段落、换行。"""
    if node is None:
        raise ValueError("缺少正文节点")
    node = copy(node)
    for text in list(node.find_all(string=True)):
        if isinstance(text, Comment):
            text.extract()
            continue
        # 源码缩进不是题干换行；只有块元素和 br 产生换行。
        value = re.sub(r"\s+", " ", text)
        text.replace_with("" if not text.strip() and "\n" in text else value)
    for tag in node.select("script, style, input, button"):
        tag.decompose()
    for tag in node.select("br"):
        tag.replace_with("\n")
    for tag in node.select("p, div, li"):
        tag.insert_before("\n")
        tag.insert_after("\n")
    return "\n".join(
        text for line in node.get_text().splitlines() if (text := normalize_text(line))
    )


_QUESTION_TYPE_NAMES = (
    "单选题|多选题|填空题|判断题|简答题|名词解释|论述题|计算题|其它|分录题|资料题|连线题|" "排序题|完型填空|阅读理解|口语题|听力题|共用选项题|测评题"
)


def question_type_label(node):
    if node is None:
        return None
    for tag in node.select("span, strong, b"):
        label = tag.get_text()
        if re.fullmatch(_QUESTION_TYPE_NAMES, normalize_text(label)):
            prefix = normalize_text(node.get_text().split(label, 1)[0])
            if re.fullmatch(r"(?:\d+\s*[.．、])?", prefix):
                return tag
    return None


def question_text(node, *, exam: bool = False) -> str:
    if node is None:
        raise ValueError("缺少题干")
    node = copy(node)
    if exam:
        for heading in node.select("h3"):
            heading.decompose()
    label = question_type_label(node)
    if label is not None:
        label.decompose()
    text = html_text(node)
    # 只剥离位于开头的题号、题型及分值元数据，避免吞掉正文中的数字或括号。
    text = re.sub(r"^\s*\d+\s*[.．、](?!\d)\s*", "", text)
    text = re.sub(
        rf"^\s*[（(【\[]\s*(?:{_QUESTION_TYPE_NAMES})"
        r"(?:\s*[,，]\s*\d+(?:\.\d+)?\s*分)?\s*[）)】\]]\s*",
        "",
        text,
    )
    text = re.sub(r"^\s*[（(]\s*\d+(?:\.\d+)?\s*分\s*[）)]\s*", "", text)
    if not text.strip():
        raise ValueError("题干为空")
    return text.strip()
