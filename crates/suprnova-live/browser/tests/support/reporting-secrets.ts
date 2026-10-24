// Every sink must remove the whole secret, including encoded names and quoted values.
export const REPORTING_SECRET_CASES = [
  { text: "Cookie: a=COOKIE_A; b=COOKIE_B", secrets: ["COOKIE_A", "COOKIE_B"] },
  { text: "Set-Cookie: b=COOKIE_C; HttpOnly; Secure", secrets: ["COOKIE_C"] },
  { text: '{"password":"WORD_ONE WORD_TWO"}', secrets: ["WORD_ONE", "WORD_TWO"] },
  { text: "password='SINGLE_ONE SINGLE_TWO'", secrets: ["SINGLE_ONE", "SINGLE_TWO"] },
  {
    text: JSON.stringify({ password: 'FIRST "QUOTED" LAST' }),
    secrets: ["FIRST", "QUOTED", "LAST"],
  },
  {
    text: "/path?%70assword=ENCODED_SECRET&api%5Fkey=SECOND_SECRET",
    secrets: ["ENCODED_SECRET", "SECOND_SECRET"],
  },
  { text: "/path?%70assword%=MALFORMED_SECRET", secrets: ["MALFORMED_SECRET"] },
  { text: "xsrf_id=XSRF_SECRET", secrets: ["XSRF_SECRET"] },
  { text: 'password="UNCLOSED_SECRET', secrets: ["UNCLOSED_SECRET"] },
  { text: "password[0]=ARRAY_SECRET", secrets: ["ARRAY_SECRET"] },
] as const;
