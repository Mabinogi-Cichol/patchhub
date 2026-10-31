(async () => {
  try {
    const origin = "https://www.patch.moe";
    if (location.origin !== origin) throw new Error("请在 Shiravune 补丁列表页面采集");
    const forms = [...document.querySelectorAll("form")].filter(form =>
      ["form_en", "form_zh_cn", "form_zh_tw"].includes(form.name));
    if (!forms.length) {
      const text = document.body?.innerText || "";
      if (text.includes("Please wait while your request is being verified") || text.includes("正在验证您的请求"))
        return {waiting: true};
      if (text.includes("Age Verification") && text.includes("18 years"))
        return {error: "source_age_verification_required"};
      return {error: "source_list_missing"};
    }
    const pages = [{url: origin + "/?flag=1", method: "GET", body: null,
      response_url: location.href, status: 200, text: document.documentElement.outerHTML}];
    for (const form of forms) {
      const url = new URL(form.getAttribute("action") || "/", location.href);
      if (url.origin !== origin || form.method.toLowerCase() !== "post")
        throw new Error("来源语言表单不符合预期");
      const fields = [...form.querySelectorAll("input[name]")]
        .filter(input => (input.getAttribute("type") || "hidden").toLowerCase() === "hidden")
        .map(input => [input.name, input.value]);
      const body = new URLSearchParams(fields).toString();
      const response = await fetch(url.href, {method: "POST", body,
        headers: {"Content-Type": "application/x-www-form-urlencoded"},
        credentials: "same-origin", mode: "same-origin"});
      if (!response.ok || new URL(response.url).origin !== origin)
        throw new Error("语言列表请求失败：" + response.status);
      pages.push({url: url.href, method: "POST", body, response_url: response.url,
        status: response.status, text: await response.text()});
    }
    return {pages};
  } catch (error) { return {error: String(error.message || error)}; }
})()
