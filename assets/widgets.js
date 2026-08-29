/* Reusable lesson widgets. Every lesson in this workspace links this file
   instead of inlining its own JavaScript.

   Two widgets:

   1. Field exercise — immediate, per-field feedback.

        <div class="exercise" data-exercise>
          <div class="fields">
            <div class="field">
              <label>word offset</label>
              <input data-answer="12|0xC">
              <span class="mark"></span>
            </div>
          </div>
          <button class="btn" data-check>检查</button>
          <button class="btn ghost" data-reveal>看答案</button>
          <p class="verdict"></p>
        </div>

      data-answer holds pipe-separated accepted values. Comparison is
      case-insensitive and whitespace-trimmed; numeric answers also match
      across decimal / 0x-hex forms so the learner can answer in either.

   2. Quiz — one correct option per question, locks after answering.

        <div class="quiz-q">
          <p>问题</p>
          <div class="opts">
            <button class="opt" data-correct>对的那条</button>
            <button class="opt">错的那条</button>
          </div>
          <p class="feedback">为什么</p>
        </div>
*/

(function () {
  "use strict";

  function normalise(raw) {
    var s = String(raw).trim().toLowerCase().replace(/\s+/g, "");
    if (s === "") return null;
    var n;
    if (/^0x[0-9a-f]+$/.test(s)) n = parseInt(s.slice(2), 16);
    else if (/^-?\d+$/.test(s)) n = parseInt(s, 10);
    else return s;
    return "#" + n;
  }

  function accepts(input, value) {
    var given = normalise(value);
    if (given === null) return false;
    return input
      .getAttribute("data-answer")
      .split("|")
      .some(function (candidate) {
        return normalise(candidate) === given;
      });
  }

  function gradeField(field) {
    var input = field.querySelector("input[data-answer]");
    if (!input) return true;
    var ok = accepts(input, input.value);
    field.classList.toggle("ok", ok);
    field.classList.toggle("bad", !ok);
    return ok;
  }

  function wireExercise(root) {
    var fields = Array.prototype.slice.call(root.querySelectorAll(".field"));
    var verdict = root.querySelector(".verdict");
    var checkBtn = root.querySelector("[data-check]");
    var revealBtn = root.querySelector("[data-reveal]");

    fields.forEach(function (field) {
      var input = field.querySelector("input[data-answer]");
      if (!input) return;
      input.addEventListener("input", function () {
        field.classList.remove("ok", "bad");
        if (verdict) {
          verdict.textContent = "";
          verdict.className = "verdict";
        }
      });
      input.addEventListener("keydown", function (event) {
        if (event.key === "Enter" && checkBtn) checkBtn.click();
      });
    });

    if (checkBtn) {
      checkBtn.addEventListener("click", function () {
        var wrong = fields.filter(function (field) {
          return !gradeField(field);
        }).length;
        if (!verdict) return;
        if (wrong === 0) {
          verdict.textContent = root.getAttribute("data-pass") || "全对。";
          verdict.className = "verdict pass";
        } else {
          verdict.textContent = wrong + " 处还不对 —— 回去看看这条描述符的第几个 dword。";
          verdict.className = "verdict fail";
        }
      });
    }

    if (revealBtn) {
      revealBtn.addEventListener("click", function () {
        fields.forEach(function (field) {
          var input = field.querySelector("input[data-answer]");
          if (!input) return;
          input.value = input.getAttribute("data-answer").split("|")[0];
          field.classList.add("ok");
          field.classList.remove("bad");
        });
        if (verdict) {
          verdict.textContent = "答案已填入。明天再回来默一遍——看懂不等于记住。";
          verdict.className = "verdict";
        }
      });
    }
  }

  function wireQuestion(question) {
    var options = Array.prototype.slice.call(question.querySelectorAll(".opt"));
    var feedback = question.querySelector(".feedback");
    options.forEach(function (option) {
      option.addEventListener("click", function () {
        if (question.dataset.answered) return;
        question.dataset.answered = "1";
        options.forEach(function (other) {
          other.classList.add("locked");
          if (other.hasAttribute("data-correct")) other.classList.add("correct");
        });
        if (!option.hasAttribute("data-correct")) option.classList.add("wrong");
        if (feedback) feedback.classList.add("show");
      });
    });
  }

  document.addEventListener("DOMContentLoaded", function () {
    document.querySelectorAll("[data-exercise]").forEach(wireExercise);
    document.querySelectorAll(".quiz-q").forEach(wireQuestion);
  });
})();
