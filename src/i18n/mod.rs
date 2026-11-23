//! Every word the bot says, in Urdu, Punjabi, Pashto, Sindhi and English.
//! All wording lives in the `Words` tables below, so a native speaker can correct a
//! translation by editing one line. Replies are short "label: value" lines, which read
//! the same way in every language.

use chrono::NaiveDate;

use crate::{
    models::{party::normalize_party_name, pending_action::Action, task::TaskWithAssignee},
    nlp::{intent::Missing, Period, TxKind},
    services::{
        ledger::{format_rupees, PartyStatement},
        reports::Digest,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Ur,
    Pa,
    Ps,
    Sd,
    En,
}

pub const ALL_LANGS: [Lang; 5] = [Lang::Ur, Lang::Pa, Lang::Ps, Lang::Sd, Lang::En];

impl Lang {
    /// Unknown codes fall back to Urdu, the most widely understood language.
    pub fn from_code(code: &str) -> Lang {
        match code {
            "pa" => Lang::Pa,
            "ps" => Lang::Ps,
            "sd" => Lang::Sd,
            "en" => Lang::En,
            _ => Lang::Ur,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Lang::Ur => "ur",
            Lang::Pa => "pa",
            Lang::Ps => "ps",
            Lang::Sd => "sd",
            Lang::En => "en",
        }
    }

    fn words(self) -> &'static Words {
        match self {
            Lang::Ur => &UR,
            Lang::Pa => &PA,
            Lang::Ps => &PS,
            Lang::Sd => &SD,
            Lang::En => &EN,
        }
    }
}

struct Words {
    rupee: &'static str,
    // kinds of entries
    sale: &'static str,
    expense: &'static str,
    received: &'static str,
    paid: &'static str,
    credit: &'static str,
    // labels
    amount: &'static str,
    name: &'static str,
    detail: &'static str,
    task: &'static str,
    assignee: &'static str,
    due_date: &'static str,
    phone: &'static str,
    // confirmation
    yes: &'static str,
    no: &'static str,
    save_q: &'static str,
    complete_q: &'static str,
    add_emp_q: &'static str,
    say_yes_no: &'static str,
    saved: &'static str,
    cancelled: &'static str,
    // money position
    cash: &'static str,
    you_get: &'static str,
    you_give: &'static str,
    owes_you: &'static str,
    you_owe: &'static str,
    settled: &'static str,
    // follow-up questions
    ask_amount: &'static str,
    ask_name: &'static str,
    ask_assignee: &'static str,
    ask_task: &'static str,
    ask_emp_name: &'static str,
    ask_emp_phone: &'static str,
    ask_language: &'static str,
    choose: &'static str,
    // messages
    didnt: &'static str,
    not_registered: &'static str,
    not_allowed: &'static str,
    nothing_confirm: &'static str,
    error: &'static str,
    lang_changed: &'static str,
    too_many: &'static str,
    no_employee: &'static str,
    no_task: &'static str,
    no_open: &'static str,
    party_unknown: &'static str,
    task_assigned: &'static str,
    notified_not: &'static str,
    task_notice: &'static str,
    task_done: &'static str,
    task_done_notice: &'static str,
    employee_added: &'static str,
    employee_taken: &'static str,
    // reports
    report_today: &'static str,
    report_week: &'static str,
    report_month: &'static str,
    profit: &'static str,
    tasks_open: &'static str,
    tasks_overdue: &'static str,
    tasks_done: &'static str,
    top_collect: &'static str,
    top_pay: &'static str,
    open_tasks_header: &'static str,
    no_tasks: &'static str,
    overdue_mark: &'static str,
    // onboarding
    welcome: &'static str,
    help: &'static str,
}

static EN: Words = Words {
    rupee: "Rs",
    sale: "Sale",
    expense: "Expense",
    received: "Received",
    paid: "Paid",
    credit: "on credit",
    amount: "Amount",
    name: "Name",
    detail: "Detail",
    task: "Task",
    assignee: "To",
    due_date: "Due",
    phone: "Phone",
    yes: "Yes",
    no: "No",
    save_q: "Save this?",
    complete_q: "Mark this task as done?",
    add_emp_q: "Add this employee?",
    say_yes_no: "Tap Yes or No, or say it by voice.",
    saved: "Saved ✅",
    cancelled: "Cancelled",
    cash: "Cash in hand",
    you_get: "Customers owe you",
    you_give: "You owe suppliers",
    owes_you: "{name} owes you {amount}",
    you_owe: "You owe {name} {amount}",
    settled: "{name}: all settled",
    ask_amount: "How much money? Please say the amount.",
    ask_name: "Which customer or supplier? Please say the name.",
    ask_assignee: "Which employee? Please say the name.",
    ask_task: "What is the task? Please say it.",
    ask_emp_name: "What is the employee’s name?",
    ask_emp_phone: "What is the employee’s mobile number? Say it digit by digit.",
    ask_language: "Which language? Say Urdu, Punjabi, Pashto, Sindhi or English.",
    choose: "Which one? Please say the full name:",
    didnt: "Sorry, I did not understand. Please say it again.",
    not_registered: "This number is not registered. Ask your employer to add you.",
    not_allowed: "Sorry, only the owner can do this.",
    nothing_confirm: "There is nothing waiting for confirmation.",
    error: "Something went wrong. Please try again.",
    lang_changed: "Language changed to English.",
    too_many: "I could not get the details. Please start again.",
    no_employee: "No employee named {name}. Add them first.",
    no_task: "I could not find that task.",
    no_open: "There are no open tasks.",
    party_unknown: "No record for {name}.",
    task_assigned: "Task given ✅",
    notified_not: "I could not message {name}. Please tell them yourself.",
    task_notice: "New task from {owner}",
    task_done: "Task finished ✅",
    task_done_notice: "{who} finished a task",
    employee_added: "{name} added ✅ They can now send voice notes here.",
    employee_taken: "This number is already registered.",
    report_today: "Today’s report",
    report_week: "Last 7 days",
    report_month: "This month",
    profit: "Profit",
    tasks_open: "Open tasks",
    tasks_overdue: "Overdue",
    tasks_done: "Finished",
    top_collect: "Most to collect",
    top_pay: "Most to pay",
    open_tasks_header: "Open tasks",
    no_tasks: "No open tasks",
    overdue_mark: "overdue",
    welcome: "Welcome to Awaaz, {name}! 🎙️\nSend a voice note in your own language.\nExample: “Sold Ali cloth worth 2500 on credit”.\nSay “help” any time.",
    help: "You can say:\n• Sales and expenses: “Sold cloth worth 2500 to Ali on credit”\n• Money received: “Ali paid 1000”\n• Tasks: “Tell Bilal to deliver to Lahore tomorrow”\n• Finished: “Bilal’s task is done”\n• Reports: “Today’s report”, “Ali’s account”, “How much cash do I have?”\n• Add staff: “Add Bilal, 0300 1234567”\n• Language: “Reply in Urdu”",
};

static UR: Words = Words {
    rupee: "روپے",
    sale: "فروخت",
    expense: "خرچہ",
    received: "وصولی",
    paid: "ادائیگی",
    credit: "ادھار",
    amount: "رقم",
    name: "نام",
    detail: "تفصیل",
    task: "کام",
    assignee: "کس کو",
    due_date: "آخری تاریخ",
    phone: "نمبر",
    yes: "ہاں",
    no: "نہیں",
    save_q: "کیا یہ محفوظ کر دوں؟",
    complete_q: "کیا یہ کام مکمل ہو گیا؟",
    add_emp_q: "کیا یہ ملازم شامل کر دوں؟",
    say_yes_no: "بٹن دبائیں، یا بول کر ہاں یا نہیں کہیں۔",
    saved: "محفوظ ہو گیا ✅",
    cancelled: "منسوخ کر دیا",
    cash: "کیش",
    you_get: "گاہکوں سے لینے ہیں",
    you_give: "سپلائرز کو دینے ہیں",
    owes_you: "{name} نے آپ کو {amount} دینے ہیں",
    you_owe: "آپ نے {name} کو {amount} دینے ہیں",
    settled: "{name}: حساب برابر ہے",
    ask_amount: "کتنی رقم؟ براہ کرم رقم بتائیں۔",
    ask_name: "کس گاہک یا سپلائر کا؟ نام بتائیں۔",
    ask_assignee: "کس ملازم کو؟ نام بتائیں۔",
    ask_task: "کام کیا ہے؟ بول کر بتائیں۔",
    ask_emp_name: "ملازم کا نام کیا ہے؟",
    ask_emp_phone: "ملازم کا موبائل نمبر بتائیں، ایک ایک ہندسہ کر کے۔",
    ask_language: "کون سی زبان؟ اردو، پنجابی، پشتو، سندھی یا انگریزی بتائیں۔",
    choose: "کون سا؟ پورا نام بتائیں:",
    didnt: "معاف کیجیے، سمجھ نہیں آئی۔ دوبارہ بول کر بتائیں۔",
    not_registered: "یہ نمبر رجسٹر نہیں ہے۔ اپنے مالک سے کہیں کہ آپ کو شامل کریں۔",
    not_allowed: "معاف کیجیے، یہ کام صرف مالک کر سکتا ہے۔",
    nothing_confirm: "تصدیق کے لیے کچھ باقی نہیں ہے۔",
    error: "کچھ مسئلہ ہو گیا۔ دوبارہ کوشش کریں۔",
    lang_changed: "زبان اردو کر دی گئی۔",
    too_many: "تفصیل پوری نہیں ہو سکی۔ دوبارہ شروع کریں۔",
    no_employee: "{name} نام کا کوئی ملازم نہیں۔ پہلے انہیں شامل کریں۔",
    no_task: "وہ کام نہیں ملا۔",
    no_open: "کوئی کھلا کام نہیں ہے۔",
    party_unknown: "{name} کا کوئی ریکارڈ نہیں۔",
    task_assigned: "کام دے دیا ✅",
    notified_not: "{name} کو پیغام نہیں جا سکا۔ انہیں خود بتا دیں۔",
    task_notice: "{owner} کی طرف سے نیا کام",
    task_done: "کام مکمل ✅",
    task_done_notice: "{who} نے کام مکمل کر دیا",
    employee_added: "{name} شامل ہو گئے ✅ اب وہ یہاں وائس نوٹ بھیج سکتے ہیں۔",
    employee_taken: "یہ نمبر پہلے سے رجسٹر ہے۔",
    report_today: "آج کی رپورٹ",
    report_week: "پچھلے 7 دن",
    report_month: "اس مہینے",
    profit: "منافع",
    tasks_open: "کھلے کام",
    tasks_overdue: "دیر ہو گئی",
    tasks_done: "مکمل",
    top_collect: "سب سے زیادہ لینے ہیں",
    top_pay: "سب سے زیادہ دینے ہیں",
    open_tasks_header: "کھلے کام",
    no_tasks: "کوئی کھلا کام نہیں",
    overdue_mark: "دیر",
    welcome: "آواز میں خوش آمدید، {name}! 🎙️\nاپنی زبان میں وائس نوٹ بھیجیں۔\nمثال: “علی کو ڈھائی ہزار کا مال ادھار بیچا”۔\nکبھی بھی “مدد” بولیں۔",
    help: "آپ یہ بول سکتے ہیں:\n• بکری اور خرچہ: “علی کو ڈھائی ہزار کا کپڑا ادھار بیچا”\n• وصولی: “علی سے ہزار روپے مل گئے”\n• کام: “بلال کو کہو کل لاہور مال پہنچائے”\n• مکمل: “بلال کا کام ہو گیا”\n• رپورٹ: “آج کا حساب”، “علی کا حساب”، “کتنے پیسے ہیں؟”\n• ملازم: “بلال کو شامل کرو، 0300 1234567”\n• زبان: “پشتو میں جواب دو”",
};

static PA: Words = Words {
    rupee: "روپے",
    sale: "وکری",
    expense: "خرچہ",
    received: "وصولی",
    paid: "ادائیگی",
    credit: "اُدھار",
    amount: "رقم",
    name: "ناں",
    detail: "تفصیل",
    task: "کم",
    assignee: "کس نوں",
    due_date: "آخری تریخ",
    phone: "نمبر",
    yes: "ہاں",
    no: "نہیں",
    save_q: "کی ایہہ محفوظ کر دیاں؟",
    complete_q: "کی ایہہ کم مکمل ہو گیا؟",
    add_emp_q: "کی ایہہ ملازم شامل کر دیاں؟",
    say_yes_no: "بٹن دباؤ، یا بول کے ہاں یا نہیں آکھو۔",
    saved: "محفوظ ہو گیا ✅",
    cancelled: "منسوخ کر دتا",
    cash: "کیش",
    you_get: "گاہکاں توں لینے نیں",
    you_give: "سپلائراں نوں دینے نیں",
    owes_you: "{name} نے تہانوں {amount} دینے نیں",
    you_owe: "تسیں {name} نوں {amount} دینے نیں",
    settled: "{name}: حساب برابر اے",
    ask_amount: "کنی رقم؟ مہربانی کر کے رقم دسو۔",
    ask_name: "کس گاہک یا سپلائر دا؟ ناں دسو۔",
    ask_assignee: "کس ملازم نوں؟ ناں دسو۔",
    ask_task: "کم کیہ اے؟ بول کے دسو۔",
    ask_emp_name: "ملازم دا ناں کیہ اے؟",
    ask_emp_phone: "ملازم دا موبائل نمبر دسو، اک اک ہندسہ کر کے۔",
    ask_language: "کہڑی زبان؟ اردو، پنجابی، پشتو، سندھی یا انگریزی دسو۔",
    choose: "کہڑا؟ پورا ناں دسو:",
    didnt: "معاف کرنا، سمجھ نہیں آئی۔ دوبارہ بول کے دسو۔",
    not_registered: "ایہہ نمبر رجسٹر نہیں۔ اپنے مالک نوں آکھو کہ تہانوں شامل کرے۔",
    not_allowed: "معاف کرنا، ایہہ کم صرف مالک کر سکدا اے۔",
    nothing_confirm: "تصدیق لئی کجھ نہیں رہندا۔",
    error: "کجھ گڑبڑ ہو گئی۔ دوبارہ کوشش کرو۔",
    lang_changed: "زبان پنجابی کر دتی گئی۔",
    too_many: "تفصیل پوری نہیں ہو سکی۔ دوبارہ شروع کرو۔",
    no_employee: "{name} ناں دا کوئی ملازم نہیں۔ پہلاں اوہنوں شامل کرو۔",
    no_task: "اوہ کم نہیں لبھیا۔",
    no_open: "کوئی کھلا کم نہیں۔",
    party_unknown: "{name} دا کوئی ریکارڈ نہیں۔",
    task_assigned: "کم دے دتا ✅",
    notified_not: "{name} نوں پیغام نہیں جا سکیا۔ اوہنوں آپ دس دیو۔",
    task_notice: "{owner} ولوں نواں کم",
    task_done: "کم مکمل ✅",
    task_done_notice: "{who} نے کم مکمل کر دتا",
    employee_added: "{name} شامل ہو گئے ✅ ہون اوہ ایتھے وائس نوٹ گھل سکدے نیں۔",
    employee_taken: "ایہہ نمبر پہلاں توں رجسٹر اے۔",
    report_today: "اج دی رپورٹ",
    report_week: "پچھلے 7 دن",
    report_month: "ایس مہینے",
    profit: "منافع",
    tasks_open: "کھلے کم",
    tasks_overdue: "دیر ہو گئی",
    tasks_done: "مکمل",
    top_collect: "سب توں ودھ لینے نیں",
    top_pay: "سب توں ودھ دینے نیں",
    open_tasks_header: "کھلے کم",
    no_tasks: "کوئی کھلا کم نہیں",
    overdue_mark: "دیر",
    welcome: "آواز وچ جی آیاں نوں، {name}! 🎙️\nاپنی زبان وچ وائس نوٹ گھلو۔\nمثال: “علی نوں اڑھائی ہزار دا مال اُدھار ویچیا”۔\nکدے وی “مدد” بولو۔",
    help: "تسیں ایہہ بول سکدے او:\n• وکری تے خرچہ: “علی نوں اڑھائی ہزار دا کپڑا اُدھار ویچیا”\n• وصولی: “علی توں ہزار روپے مل گئے”\n• کم: “بلال نوں آکھو کل لاہور مال پہنچائے”\n• مکمل: “بلال دا کم ہو گیا”\n• رپورٹ: “اج دا حساب”، “علی دا حساب”، “کنے پیسے نیں؟”\n• ملازم: “بلال نوں شامل کرو، 0300 1234567”\n• زبان: “اردو وچ جواب دیو”",
};

static PS: Words = Words {
    rupee: "روپۍ",
    sale: "خرڅلاو",
    expense: "لګښت",
    received: "ترلاسه",
    paid: "تادیه",
    credit: "په پور",
    amount: "رقم",
    name: "نوم",
    detail: "تفصیل",
    task: "کار",
    assignee: "چا ته",
    due_date: "وروستۍ نېټه",
    phone: "شمېره",
    yes: "هو",
    no: "نه",
    save_q: "ایا دا ثبت کړم؟",
    complete_q: "ایا دا کار بشپړ شو؟",
    add_emp_q: "ایا دا کارمند اضافه کړم؟",
    say_yes_no: "تڼۍ وټوکئ، یا په غږ ووایاست هو یا نه۔",
    saved: "ثبت شو ✅",
    cancelled: "لغوه شو",
    cash: "نغدې پیسې",
    you_get: "له پیرودونکو اخیستل",
    you_give: "سپلائرانو ته ورکول",
    owes_you: "{name} تاسو ته {amount} پور دی",
    you_owe: "تاسو {name} ته {amount} پور یاست",
    settled: "{name}: حساب برابر دی",
    ask_amount: "څومره پیسې؟ مهرباني وکړئ رقم ووایاست۔",
    ask_name: "کوم پیرودونکی یا سپلائر؟ نوم ووایاست۔",
    ask_assignee: "کوم کارمند ته؟ نوم ووایاست۔",
    ask_task: "کار څه دی؟ ووایاست۔",
    ask_emp_name: "د کارمند نوم څه دی؟",
    ask_emp_phone: "د کارمند موبایل شمېره ووایاست، یو یو عدد۔",
    ask_language: "کومه ژبه؟ اردو، پنجابي، پښتو، سندهي یا انګلیسي ووایاست۔",
    choose: "کوم یو؟ بشپړ نوم ووایاست:",
    didnt: "بخښنه، پوه نه شوم. بیا ووایاست۔",
    not_registered: "دا شمېره ثبت نه ده. له خپل مالک وغواړئ چې تاسو اضافه کړي۔",
    not_allowed: "بخښنه، دا کار یوازې مالک کولای شي۔",
    nothing_confirm: "د تصدیق لپاره هیڅ نشته۔",
    error: "یوه ستونزه رامنځته شوه. بیا هڅه وکړئ۔",
    lang_changed: "ژبه پښتو شوه۔",
    too_many: "جزئیات بشپړ نه شول. بیا پیل کړئ۔",
    no_employee: "د {name} په نوم کارمند نشته. لومړی یې اضافه کړئ۔",
    no_task: "هغه کار ونه موندل شو۔",
    no_open: "هیڅ پرانیستی کار نشته۔",
    party_unknown: "د {name} هیڅ ریکارډ نشته۔",
    task_assigned: "کار ورکړل شو ✅",
    notified_not: "{name} ته پیغام ونه رسېد. ورته پخپله ووایاست۔",
    task_notice: "د {owner} لخوا نوی کار",
    task_done: "کار بشپړ شو ✅",
    task_done_notice: "{who} کار بشپړ کړ",
    employee_added: "{name} اضافه شو ✅ اوس هغه دلته غږیز پیغامونه لېږلای شي۔",
    employee_taken: "دا شمېره دمخه ثبت ده۔",
    report_today: "د نن راپور",
    report_week: "تېر ۷ ورځې",
    report_month: "دا میاشت",
    profit: "ګټه",
    tasks_open: "پرانیستي کارونه",
    tasks_overdue: "ځنډېدلي",
    tasks_done: "بشپړ",
    top_collect: "تر ټولو ډېر اخیستل",
    top_pay: "تر ټولو ډېر ورکول",
    open_tasks_header: "پرانیستي کارونه",
    no_tasks: "هیڅ پرانیستی کار نشته",
    overdue_mark: "ځنډ",
    welcome: "آواز ته ښه راغلاست، {name}! 🎙️\nپه خپله ژبه غږیز پیغام ولېږئ۔\nمثال: “علي ته په پور د دوه نیم زره جامې وپلورلې”۔\nهر وخت “مرسته” ووایاست۔",
    help: "تاسو دا ویلای شئ:\n• پلور او لګښت: “علي ته په پور د دوه نیم زره جامې وپلورلې”\n• ترلاسه: “له علي څخه زر روپۍ ترلاسه شوې”\n• کار: “بلال ته ووایه سبا لاهور ته مال ورسوي”\n• بشپړ: “د بلال کار بشپړ شو”\n• راپور: “د نن حساب”، “د علي حساب”، “څومره پیسې دي؟”\n• کارمند: “بلال اضافه کړه، 0300 1234567”\n• ژبه: “په اردو ځواب ورکړه”",
};

static SD: Words = Words {
    rupee: "رپيا",
    sale: "وڪرو",
    expense: "خرچ",
    received: "وصولي",
    paid: "ادائگي",
    credit: "ادھار",
    amount: "رقم",
    name: "نالو",
    detail: "تفصيل",
    task: "ڪم",
    assignee: "ڪنهن کي",
    due_date: "آخري تاريخ",
    phone: "نمبر",
    yes: "ها",
    no: "نه",
    save_q: "ڇا هي محفوظ ڪريان؟",
    complete_q: "ڇا هي ڪم مڪمل ٿيو؟",
    add_emp_q: "ڇا هي ملازم شامل ڪريان؟",
    say_yes_no: "بٽن دٻايو، يا ٻول ڪري ها يا نه چئو.",
    saved: "محفوظ ٿي ويو ✅",
    cancelled: "منسوخ ڪيو ويو",
    cash: "نقد",
    you_get: "گراهڪن کان وٺڻا آهن",
    you_give: "سپلائرز کي ڏيڻا آهن",
    owes_you: "{name} توهان کي {amount} ڏيڻا آهن",
    you_owe: "توهان {name} کي {amount} ڏيڻا آهن",
    settled: "{name}: حساب برابر آهي",
    ask_amount: "ڪيتري رقم؟ مهرباني ڪري رقم ٻڌايو.",
    ask_name: "ڪهڙي گراهڪ يا سپلائر جو؟ نالو ٻڌايو.",
    ask_assignee: "ڪهڙي ملازم کي؟ نالو ٻڌايو.",
    ask_task: "ڪم ڇا آهي؟ ٻول ڪري ٻڌايو.",
    ask_emp_name: "ملازم جو نالو ڇا آهي؟",
    ask_emp_phone: "ملازم جو موبائل نمبر ٻڌايو، هڪ هڪ انگ ڪري.",
    ask_language: "ڪهڙي ٻولي؟ اردو، پنجابي، پشتو، سنڌي يا انگريزي ٻڌايو.",
    choose: "ڪهڙو؟ پورو نالو ٻڌايو:",
    didnt: "معاف ڪجو، سمجهه ۾ نه آيو. ٻيهر ٻڌايو.",
    not_registered: "هي نمبر رجسٽر ٿيل ناهي. پنهنجي مالڪ کي چئو ته توهان کي شامل ڪري.",
    not_allowed: "معاف ڪجو، هي ڪم صرف مالڪ ڪري سگهي ٿو.",
    nothing_confirm: "تصديق لاءِ ڪجهه ناهي.",
    error: "ڪو مسئلو ٿي ويو. ٻيهر ڪوشش ڪريو.",
    lang_changed: "ٻولي سنڌي ڪئي وئي.",
    too_many: "تفصيل پورو نه ٿي سگهيو. ٻيهر شروع ڪريو.",
    no_employee: "{name} نالي ڪو ملازم ناهي. پهرين کيس شامل ڪريو.",
    no_task: "اهو ڪم نه مليو.",
    no_open: "ڪو کليل ڪم ناهي.",
    party_unknown: "{name} جو ڪو رڪارڊ ناهي.",
    task_assigned: "ڪم ڏنو ويو ✅",
    notified_not: "{name} کي پيغام نه وڃي سگهيو. کيس پاڻ ٻڌايو.",
    task_notice: "{owner} طرفان نئون ڪم",
    task_done: "ڪم مڪمل ✅",
    task_done_notice: "{who} ڪم مڪمل ڪيو",
    employee_added: "{name} شامل ٿي ويو ✅ هاڻي هو هتي وائس نوٽ موڪلي سگهي ٿو.",
    employee_taken: "هي نمبر اڳ ۾ ئي رجسٽر ٿيل آهي.",
    report_today: "اڄ جي رپورٽ",
    report_week: "گذريل 7 ڏينهن",
    report_month: "هن مهيني",
    profit: "منافعو",
    tasks_open: "کليل ڪم",
    tasks_overdue: "دير ٿي ويل",
    tasks_done: "مڪمل",
    top_collect: "سڀ کان وڌيڪ وٺڻا",
    top_pay: "سڀ کان وڌيڪ ڏيڻا",
    open_tasks_header: "کليل ڪم",
    no_tasks: "ڪو کليل ڪم ناهي",
    overdue_mark: "دير",
    welcome: "آواز ۾ ڀليڪار، {name}! 🎙️\nپنهنجي ٻولي ۾ وائس نوٽ موڪليو.\nمثال: “علي کي اڍائي هزار جو مال ادھار وڪڻيو”.\nڪڏهن به “مدد” چئو.",
    help: "توهان هي چئي سگهو ٿا:\n• وڪرو ۽ خرچ: “علي کي اڍائي هزار جو ڪپڙو ادھار وڪڻيو”\n• وصولي: “علي کان هزار رپيا مليا”\n• ڪم: “بلال کي چئو سڀاڻي لاهور مال پهچائي”\n• مڪمل: “بلال جو ڪم ٿي ويو”\n• رپورٽ: “اڄ جو حساب”، “علي جو حساب”، “ڪيترا پيسا آهن؟”\n• ملازم: “بلال کي شامل ڪريو، 0300 1234567”\n• ٻولي: “اردو ۾ جواب ڏيو”",
};

// ---------- small helpers ----------

fn fill(template: &str, pairs: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (key, value) in pairs {
        out = out.replace(&format!("{{{key}}}"), value);
    }
    out
}

fn date(d: NaiveDate) -> String {
    d.format("%d-%m-%Y").to_string()
}

/// "Rs 2,500" in English; "2,500 روپے" and so on in the other languages.
pub fn money(l: Lang, paisa: i64) -> String {
    let english = format_rupees(paisa);
    match l {
        Lang::En => english,
        _ => {
            let number = english.replacen("Rs ", "", 1);
            format!("{number} {}", l.words().rupee)
        }
    }
}

fn kind_label(w: &Words, kind: TxKind) -> &'static str {
    match kind {
        TxKind::Sale => w.sale,
        TxKind::Expense => w.expense,
        TxKind::PaymentReceived => w.received,
        TxKind::PaymentMade => w.paid,
    }
}

fn credit_tag(w: &Words, on_credit: bool) -> String {
    if on_credit {
        format!(" ({})", w.credit)
    } else {
        String::new()
    }
}

/// "Ali owes you Rs 1,500", "You owe Ali Rs 1,500" or "Ali: all settled".
fn balance_line(l: Lang, name: &str, balance_paisa: i64) -> String {
    let w = l.words();
    if balance_paisa > 0 {
        fill(w.owes_you, &[("name", name), ("amount", &money(l, balance_paisa))])
    } else if balance_paisa < 0 {
        fill(w.you_owe, &[("name", name), ("amount", &money(l, -balance_paisa))])
    } else {
        fill(w.settled, &[("name", name)])
    }
}

// ---------- plain messages ----------

pub fn yes_label(l: Lang) -> &'static str {
    l.words().yes
}

pub fn no_label(l: Lang) -> &'static str {
    l.words().no
}

pub fn say_yes_no(l: Lang) -> &'static str {
    l.words().say_yes_no
}

pub fn welcome(l: Lang, name: &str) -> String {
    fill(l.words().welcome, &[("name", name)])
}

pub fn help(l: Lang) -> String {
    l.words().help.to_string()
}

pub fn not_registered(l: Lang) -> String {
    l.words().not_registered.to_string()
}

pub fn didnt_understand(l: Lang) -> String {
    l.words().didnt.to_string()
}

pub fn not_allowed(l: Lang) -> String {
    l.words().not_allowed.to_string()
}

pub fn nothing_to_confirm(l: Lang) -> String {
    l.words().nothing_confirm.to_string()
}

pub fn error(l: Lang) -> String {
    l.words().error.to_string()
}

pub fn cancelled(l: Lang) -> String {
    l.words().cancelled.to_string()
}

pub fn language_changed(l: Lang) -> String {
    l.words().lang_changed.to_string()
}

pub fn too_many_tries(l: Lang) -> String {
    l.words().too_many.to_string()
}

pub fn employee_not_found(l: Lang, name: &str) -> String {
    fill(l.words().no_employee, &[("name", name)])
}

pub fn task_no_match(l: Lang) -> String {
    l.words().no_task.to_string()
}

pub fn no_open_tasks(l: Lang) -> String {
    l.words().no_open.to_string()
}

pub fn party_unknown(l: Lang, name: &str) -> String {
    fill(l.words().party_unknown, &[("name", name)])
}

pub fn employee_taken(l: Lang) -> String {
    l.words().employee_taken.to_string()
}

pub fn employee_added(l: Lang, name: &str) -> String {
    fill(l.words().employee_added, &[("name", name)])
}

pub fn task_done(l: Lang, title: &str) -> String {
    format!("{}\n{}: {}", l.words().task_done, l.words().task, title)
}

pub fn task_done_notice(l: Lang, who: &str, title: &str) -> String {
    format!(
        "{}\n{}: {}",
        fill(l.words().task_done_notice, &[("who", who)]),
        l.words().task,
        title
    )
}

pub fn task_assigned(l: Lang, assignee: &str, notified: bool) -> String {
    let w = l.words();
    if notified {
        format!("{} ({})", w.task_assigned, assignee)
    } else {
        format!(
            "{} ({})\n{}",
            w.task_assigned,
            assignee,
            fill(w.notified_not, &[("name", assignee)])
        )
    }
}

/// Sent to the employee when the owner gives them work.
pub fn task_notice(l: Lang, owner: &str, title: &str, due: Option<NaiveDate>) -> String {
    let w = l.words();
    let mut lines = vec![
        fill(w.task_notice, &[("owner", owner)]),
        format!("{}: {}", w.task, title),
    ];
    if let Some(d) = due {
        lines.push(format!("{}: {}", w.due_date, date(d)));
    }
    lines.join("\n")
}

/// The question asked when something is missing.
pub fn ask(l: Lang, missing: Missing) -> String {
    let w = l.words();
    let text = match missing {
        Missing::Amount => w.ask_amount,
        Missing::PartyName => w.ask_name,
        Missing::Assignee => w.ask_assignee,
        Missing::TaskTitle => w.ask_task,
        Missing::EmployeeName => w.ask_emp_name,
        Missing::EmployeePhone => w.ask_emp_phone,
        Missing::Language => w.ask_language,
    };
    text.to_string()
}

/// "Which one?" followed by a numbered list of the possible names.
pub fn choose_one(l: Lang, names: &[String]) -> String {
    let mut lines = vec![l.words().choose.to_string()];
    for (i, name) in names.iter().enumerate() {
        lines.push(format!("{}. {}", i + 1, name));
    }
    lines.join("\n")
}

// ---------- read-back and results ----------

/// What the bot reads back before saving. The user answers Yes or No.
pub fn confirm_text(l: Lang, action: &Action) -> String {
    let w = l.words();
    match action {
        Action::Record {
            kind,
            amount_paisa,
            on_credit,
            party_name,
            new_party,
            description,
        } => {
            let mut lines = vec![
                format!("{}{}", kind_label(w, *kind), credit_tag(w, *on_credit)),
                format!("{}: {}", w.amount, money(l, *amount_paisa)),
            ];
            if let Some(name) = party_name {
                let mark = if *new_party { " 🆕" } else { "" };
                lines.push(format!("{}: {}{}", w.name, name, mark));
            }
            if let Some(text) = description {
                lines.push(format!("{}: {}", w.detail, text));
            }
            lines.push(String::new());
            lines.push(w.save_q.to_string());
            lines.join("\n")
        }
        Action::Assign {
            assignee_name,
            title,
            due_date,
            ..
        } => {
            let mut lines = vec![
                format!("{}: {}", w.task, title),
                format!("{}: {}", w.assignee, assignee_name),
            ];
            if let Some(d) = due_date {
                lines.push(format!("{}: {}", w.due_date, date(*d)));
            }
            lines.push(String::new());
            lines.push(w.save_q.to_string());
            lines.join("\n")
        }
        Action::Complete {
            title,
            assignee_name,
            ..
        } => format!(
            "{}: {}\n{}: {}\n\n{}",
            w.task, title, w.assignee, assignee_name, w.complete_q
        ),
        Action::AddEmployee { name, phone } => format!(
            "{}: {}\n{}: {}\n\n{}",
            w.name, name, w.phone, phone, w.add_emp_q
        ),
    }
}

/// The reply after an entry is saved.
pub fn recorded(
    l: Lang,
    kind: TxKind,
    amount_paisa: i64,
    on_credit: bool,
    party: Option<(&str, i64)>,
    cash_paisa: i64,
) -> String {
    let w = l.words();
    let mut lines = vec![
        w.saved.to_string(),
        format!(
            "{}{}: {}",
            kind_label(w, kind),
            credit_tag(w, on_credit),
            money(l, amount_paisa)
        ),
    ];
    if let Some((name, balance)) = party {
        lines.push(balance_line(l, name, balance));
    }
    lines.push(format!("{}: {}", w.cash, money(l, cash_paisa)));
    lines.join("\n")
}

pub fn balance(l: Lang, cash_paisa: i64, receivable_paisa: i64, payable_paisa: i64) -> String {
    let w = l.words();
    [
        format!("{}: {}", w.cash, money(l, cash_paisa)),
        format!("{}: {}", w.you_get, money(l, receivable_paisa)),
        format!("{}: {}", w.you_give, money(l, payable_paisa)),
    ]
    .join("\n")
}

pub fn party_statement(l: Lang, statement: &PartyStatement) -> String {
    let w = l.words();
    let mut lines = vec![balance_line(
        l,
        &statement.party.name,
        statement.party.balance_paisa,
    )];
    for entry in statement.entries.iter().take(5) {
        let kind = entry
            .kind_enum()
            .map(|k| kind_label(w, k))
            .unwrap_or_default();
        lines.push(format!(
            "{} {}{} {}",
            date(entry.occurred_on),
            kind,
            credit_tag(w, entry.on_credit),
            money(l, entry.amount_paisa)
        ));
    }
    lines.join("\n")
}

pub fn task_list(l: Lang, tasks: &[TaskWithAssignee], today: NaiveDate) -> String {
    let w = l.words();
    if tasks.is_empty() {
        return w.no_tasks.to_string();
    }

    let mut lines = vec![format!("{}:", w.open_tasks_header)];
    for (i, item) in tasks.iter().take(10).enumerate() {
        let mut line = format!("{}. {} — {}", i + 1, item.task.title, item.assignee_name);
        if let Some(due) = item.task.due_date {
            line.push_str(&format!(" ({})", date(due)));
        }
        if item.task.is_overdue(today) {
            line.push_str(&format!(" ⚠ {}", w.overdue_mark));
        }
        lines.push(line);
    }
    lines.join("\n")
}

pub fn digest(l: Lang, d: &Digest, period: Period) -> String {
    let w = l.words();
    let s = &d.summary;
    let t = &d.tasks;

    let title = match period {
        Period::Today => w.report_today,
        Period::Week => w.report_week,
        Period::Month => w.report_month,
    };
    let range = if s.from == s.to {
        date(s.from)
    } else {
        format!("{} → {}", date(s.from), date(s.to))
    };

    let mut lines = vec![
        format!("{title} ({range})"),
        format!(
            "{}: {} ({}: {})",
            w.sale,
            money(l, s.sales_paisa),
            w.credit,
            money(l, s.credit_sales_paisa)
        ),
        format!(
            "{}: {} ({}: {})",
            w.expense,
            money(l, s.expenses_paisa),
            w.credit,
            money(l, s.credit_expenses_paisa)
        ),
        format!("{}: {}", w.received, money(l, s.received_paisa)),
        format!("{}: {}", w.paid, money(l, s.paid_paisa)),
        format!("{}: {}", w.profit, money(l, s.profit_paisa())),
        format!("{}: {}", w.cash, money(l, s.cash_balance_paisa)),
        format!("{}: {}", w.you_get, money(l, s.receivable_paisa)),
        format!("{}: {}", w.you_give, money(l, s.payable_paisa)),
        format!(
            "{}: {} | {}: {} | {}: {}",
            w.tasks_open, t.open, w.tasks_overdue, t.overdue, w.tasks_done, t.done_in_range
        ),
    ];

    if !d.top_receivables.is_empty() {
        lines.push(format!(
            "{}: {}",
            w.top_collect,
            party_list(l, &d.top_receivables)
        ));
    }
    if !d.top_payables.is_empty() {
        lines.push(format!("{}: {}", w.top_pay, party_list(l, &d.top_payables)));
    }
    lines.join("\n")
}

fn party_list(l: Lang, parties: &[crate::models::party::Party]) -> String {
    parties
        .iter()
        .map(|p| format!("{} {}", p.name, money(l, p.balance_paisa.abs())))
        .collect::<Vec<_>>()
        .join("; ")
}

// ---------- yes / no by voice ----------

const YES_WORDS: &[&str] = &[
    "yes", "y", "ok", "okay", "ji", "jee", "haan", "han", "haa", "ha", "haanji", "theek", "thik",
    "sahi", "karo", "kardo", "ہاں", "ہا", "جی", "ٹھیک", "صحیح", "اوکے", "کرو", "کردو", "آہو",
    "اہو", "ہو", "ها", "جي", "ٺيڪ", "هائو", "هو", "ٹیک",
];

const NO_WORDS: &[&str] = &[
    "no", "n", "nahi", "nahin", "nai", "na", "nahee", "cancel", "stop", "نہیں", "نہی", "نہ", "نا",
    "نئیں", "نیں", "منسوخ", "کینسل", "ختم", "نه", "نھیں",
];

const FILLER_WORDS: &[&str] = &["hai", "hain", "ہے", "ہیں", "اے", "آهي", "دی"];

fn normalized_set(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| normalize_party_name(w)).collect()
}

/// Is a short reply a Yes or a No? Works for typed text and for voice transcripts in any
/// of the supported languages. Anything longer than four words, or anything unrecognised,
/// is not an answer, so the bot treats it as a new command.
pub fn parse_yes_no(text: &str) -> Option<bool> {
    // "ji" is a polite particle: "haan ji" is yes, "nahin ji" is no, and "ji" alone is yes.
    // The Urdu and Sindhi spellings both normalize to the same letters.
    const POLITE_WORDS: &[&str] = &["ji", "jee", "\u{062C}\u{06CC}"];

    let normalized = normalize_party_name(text);
    let cleaned: String = normalized
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();

    let fillers = normalized_set(FILLER_WORDS);
    let polite = normalized_set(POLITE_WORDS);

    let tokens: Vec<&str> = cleaned
        .split_whitespace()
        .filter(|token| !fillers.iter().any(|f| f == token))
        .collect();

    // Drop the polite particle when a real answer stands next to it.
    let without_polite: Vec<&str> = tokens
        .iter()
        .copied()
        .filter(|token| !polite.iter().any(|p| p == token))
        .collect();
    let tokens = if without_polite.is_empty() {
        tokens
    } else {
        without_polite
    };

    if tokens.is_empty() || tokens.len() > 4 {
        return None;
    }

    let yes = normalized_set(YES_WORDS);
    let no = normalized_set(NO_WORDS);

    if tokens.iter().all(|t| yes.iter().any(|y| y == t)) {
        Some(true)
    } else if tokens.iter().all(|t| no.iter().any(|n| n == t)) {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        models::party::Party,
        services::{
            ledger::Summary,
            reports::TaskStats,
        },
    };
    use chrono::Utc;
    use uuid::Uuid;

    #[test]
    fn language_codes_round_trip() {
        for l in ALL_LANGS {
            assert_eq!(Lang::from_code(l.code()), l);
        }
        assert_eq!(Lang::from_code("hi"), Lang::Ur);
    }

    #[test]
    fn templates_keep_their_placeholders() {
        for l in ALL_LANGS {
            let w = l.words();
            assert!(w.owes_you.contains("{name}") && w.owes_you.contains("{amount}"), "{l:?} owes_you");
            assert!(w.you_owe.contains("{name}") && w.you_owe.contains("{amount}"), "{l:?} you_owe");
            assert!(w.settled.contains("{name}"), "{l:?} settled");
            assert!(w.no_employee.contains("{name}"), "{l:?} no_employee");
            assert!(w.party_unknown.contains("{name}"), "{l:?} party_unknown");
            assert!(w.notified_not.contains("{name}"), "{l:?} notified_not");
            assert!(w.employee_added.contains("{name}"), "{l:?} employee_added");
            assert!(w.welcome.contains("{name}"), "{l:?} welcome");
            assert!(w.task_notice.contains("{owner}"), "{l:?} task_notice");
            assert!(w.task_done_notice.contains("{who}"), "{l:?} task_done_notice");
        }
    }

    #[test]
    fn button_titles_fit_whatsapps_limit() {
        for l in ALL_LANGS {
            assert!(l.words().yes.chars().count() <= 20);
            assert!(l.words().no.chars().count() <= 20);
        }
    }

    #[test]
    fn non_english_texts_really_contain_arabic_script() {
        // Guards against a terminal that mangled the pasted text into question marks.
        for l in [Lang::Ur, Lang::Pa, Lang::Ps, Lang::Sd] {
            let w = l.words();
            for text in [w.yes, w.no, w.didnt, w.welcome, w.help, w.rupee] {
                assert!(
                    text.chars().any(|c| ('\u{0600}'..='\u{06FF}').contains(&c)),
                    "{l:?} text has no Arabic-script letters: {text}"
                );
            }
        }
    }

    #[test]
    fn formats_money_per_language() {
        assert_eq!(money(Lang::En, 250_000), "Rs 2,500");
        assert_eq!(money(Lang::En, -5_000), "-Rs 50");
        assert_eq!(money(Lang::Ur, 250_000), format!("2,500 {}", UR.rupee));
        assert_eq!(money(Lang::Ps, 150_075), format!("1,500.75 {}", PS.rupee));
    }

    #[test]
    fn understands_yes_and_no_replies() {
        for yes in ["yes", "Haan ji", "ji", "OK", "theek hai", "ہاں", "ہاں جی۔", "ٹھیک ہے", "جی", "ها", "هو"] {
            assert_eq!(parse_yes_no(yes), Some(true), "{yes}");
        }
        for no in ["no", "nahi", "Nahin ji", "cancel", "نہیں", "نہ", "نه", "نہیں جی"] {
            assert_eq!(parse_yes_no(no), Some(false), "{no}");
        }
        for other in ["", "   ", "ali ko paanch sau ka maal becha", "haan nahi", "aaj ka hisab"] {
            assert_eq!(parse_yes_no(other), None, "{other}");
        }
    }

    #[test]
    fn confirmation_reads_back_the_details() {
        let action = Action::Record {
            kind: TxKind::Sale,
            amount_paisa: 250_000,
            on_credit: true,
            party_name: Some("Ali".to_string()),
            new_party: true,
            description: Some("cloth".to_string()),
        };
        let text = confirm_text(Lang::En, &action);
        assert!(text.starts_with("Sale (on credit)"));
        assert!(text.contains("Amount: Rs 2,500"));
        assert!(text.contains("Name: Ali 🆕"));
        assert!(text.contains("Detail: cloth"));
        assert!(text.ends_with("Save this?"));

        let urdu = confirm_text(Lang::Ur, &action);
        assert!(urdu.contains("2,500"));
        assert!(urdu.contains("Ali"));
    }

    #[test]
    fn saved_message_shows_party_balance_and_cash() {
        let text = recorded(Lang::En, TxKind::Sale, 250_000, true, Some(("Ali", 250_000)), 100_000);
        assert!(text.contains("Saved"));
        assert!(text.contains("Ali owes you Rs 2,500"));
        assert!(text.contains("Cash in hand: Rs 1,000"));

        let paid = recorded(Lang::En, TxKind::PaymentReceived, 250_000, false, Some(("Ali", 0)), 350_000);
        assert!(paid.contains("Ali: all settled"));

        let owed = recorded(Lang::En, TxKind::Expense, 50_000, true, Some(("Supplier", -50_000)), 0);
        assert!(owed.contains("You owe Supplier Rs 500"));
    }

    #[test]
    fn digest_renders_in_english_and_urdu() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 22).unwrap();
        let digest_data = Digest {
            summary: Summary {
                from: day,
                to: day,
                entries: 2,
                sales_paisa: 350_000,
                credit_sales_paisa: 250_000,
                expenses_paisa: 30_000,
                credit_expenses_paisa: 0,
                received_paisa: 100_000,
                paid_paisa: 0,
                cash_balance_paisa: 170_000,
                receivable_paisa: 150_000,
                payable_paisa: 0,
            },
            tasks: TaskStats {
                open: 2,
                overdue: 1,
                done_in_range: 3,
                created_in_range: 4,
            },
            top_receivables: vec![Party {
                id: Uuid::new_v4(),
                business_id: Uuid::new_v4(),
                name: "Ali".to_string(),
                balance_paisa: 150_000,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }],
            top_payables: vec![],
        };

        let en = digest(Lang::En, &digest_data, Period::Today);
        assert!(en.starts_with("Today’s report (22-11-2026)"));
        assert!(en.contains("Sale: Rs 3,500 (on credit: Rs 2,500)"));
        assert!(en.contains("Profit: Rs 3,200"));
        assert!(en.contains("Most to collect: Ali Rs 1,500"));

        let ur = digest(Lang::Ur, &digest_data, Period::Today);
        assert!(ur.contains("3,500"));
        assert!(ur.contains(UR.rupee));
    }

    #[test]
    fn choices_are_numbered() {
        let text = choose_one(Lang::En, &["Ali Khan".to_string(), "Ali Raza".to_string()]);
        assert!(text.contains("1. Ali Khan"));
        assert!(text.contains("2. Ali Raza"));
    }
}