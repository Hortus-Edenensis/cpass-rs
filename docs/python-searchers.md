# Python answer search and DeepSeek V4.1 Flash

`resolver/question.py` builds searchers from `config.SEARCHERS`. Use the Python registry name
`OpenAISearcher` in `config.yml`; Rust also accepts `openai-compatible` in its own runtime.
`CPASS_OPENAI_API_KEY` supplies the API key without storing it in the file.

```yaml
searchers:
  - type: OpenAISearcher
    base_url: "https://api.deepseek.com/v1"
    model: "deepseek-v4.1-flash"
    thinking: {type: enabled}
    reasoning_effort: high
    max_tokens: 8192
    response_format: {type: json_object}
    system_prompt: "只输出最终答案的 JSON 对象。"
    prompt: "题型：{type}\n题目：{value}\n{options}"
```

The official endpoint maps the requested `deepseek-v4.1-flash` alias to `deepseek-flash`.
Other gateway model names are passed through. Optional fields are omitted unless configured.
`thinking.type` accepts `enabled` or `disabled`; `reasoning_effort` accepts the documented effort
levels. Contradictory controls and invalid token budgets fail during configuration.

The pinned OpenAI SDK sends DeepSeek extension fields through `extra_body`. JSON mode adds
an answer-object example to the system prompt. Supported final answers include
`{"answer":"A"}`, `{"answers":["A","C"]}`, `{"answer":false}`, or an ordered blank array.
Only final `message.content` is passed to the shared strict resolver. Reasoning-only or explicitly
truncated responses never become answers. Complete leading thinking wrappers and Markdown JSON
fences are stripped centrally; incomplete/embedded thinking, malformed JSON, conflicting fields,
partial multiple-choice selections, and incomplete blanks remain unresolved.

Valid prior answers are preserved; search results do not overwrite or resubmit them. Question
parsing and save/submit receipts retain the existing fail-closed final-submission checks.

Official references, verified for the 2026-10-08 release:

- [Model updates](https://api-docs.deepseek.com/updates/)
- [Chat Completions schema](https://api-docs.deepseek.com/api/create-chat-completion/)
- [Thinking mode](https://api-docs.deepseek.com/guides/thinking_mode/)

The tests mock model requests and Chaoxing receipts. Real service availability, task completion,
and grading remain separate unverified outcomes.
