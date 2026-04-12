DROP TABLE IF EXISTS question;

CREATE TABLE question (
    question TEXT NOT NULL,
    answer TEXT NOT NULL
);

INSERT INTO question (question, answer) VALUES
    ('普通话以哪种方言为基础方言？', 'B'),
    ('[填空题 / fill_blank] 请补全“词汇”相关术语。
Blank 1: 词义：
Blank 2: 词性：', '语义内容'),
    ('[填空题 / fill_blank] 请补全“词汇”相关术语。
Blank 1: 词义：
Blank 2: 词性：', '名词'),
    ('现代汉语共同语就是普通话。', '对');
