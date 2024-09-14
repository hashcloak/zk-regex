use std::{cmp::max, fs::File, io::Write, path::Path};

use crate::structs::RegexAndDFA;

pub fn gen_noir_fn(
    regex_and_dfa: &RegexAndDFA,
    path: &Path,
    gen_substrs: bool,
) -> Result<(), std::io::Error> {
    let noir_fn = to_noir_fn(regex_and_dfa, gen_substrs);
    let mut file = File::create(path)?;
    file.write_all(noir_fn.as_bytes())?;
    file.flush()?;
    Ok(())
}

/// Generates Noir code based on the DFA and whether a substring should be extracted.
///
/// # Arguments
///
/// * `regex_and_dfa` - The `RegexAndDFA` struct containing the regex pattern and DFA.
/// * `gen_substrs` - A boolean indicating whether to generate substrings. Note that this can only
///                   be used in the `decomposed` setting.
///
/// # Returns
///
/// A `String` that contains the Noir code
fn to_noir_fn(regex_and_dfa: &RegexAndDFA, gen_substrs: bool) -> String {
    let accept_state_id = {
        let last_state = regex_and_dfa.dfa.states.last().expect("no last state");
        assert!(
            last_state.state_type == "accept",
            "last state is accept, right??"
        );
        last_state.state_id
    };

    const BYTE_SIZE: u32 = 256; // u8 size
    let mut lookup_table_body = String::new();

    // curr_state + char_code -> next_state
    let mut rows: Vec<(usize, u8, usize)> = vec![];


    let mut highest_state = 0;
    for state in regex_and_dfa.dfa.states.iter() {
        highest_state = max(state.state_id, highest_state);
        if state.state_type == "accept" {
            assert_eq!(state.transitions.len(), 0, "accept state has transitions");
        } else {
            assert!(state.transitions.len() > 0, "no transitions");
            for (&tran_next_state_id, tran) in &state.transitions {
                for &char_code in tran {
                    rows.push((state.state_id, char_code, tran_next_state_id));
                }
            }
        };
    }

    for (curr_state_id, char_code, next_state_id) in rows {
        lookup_table_body +=
            &format!("table[{curr_state_id} * {BYTE_SIZE} + {char_code}] = {next_state_id};\n",);
    }

    lookup_table_body = indent(&lookup_table_body);
    let table_size = BYTE_SIZE as usize * regex_and_dfa.dfa.states.len();

    // If the regex ends with `$`, use this invalid state to invalidate
    // any transitions after `$`
    let invalid_state = highest_state + 1;
    let mut end_anchor_logic = if regex_and_dfa.has_end_anchor {
        format!(
            r#"
for i in 0..{BYTE_SIZE} {{
    table[{accept_state_id} * {BYTE_SIZE} + i] = {invalid_state};
}}
            "#
        )
    } else {
        format!(
            r#"
for i in 0..{BYTE_SIZE} {{
    table[{accept_state_id} * {BYTE_SIZE} + i] = {accept_state_id};
}}
            "#
        )
    };
    end_anchor_logic = indent(&end_anchor_logic);

    let lookup_table = format!(
        r#"
comptime fn make_lookup_table() -> [Field; {table_size}] {{
    let mut table = [0; {table_size}];
{lookup_table_body}
    // experimentally confirmed that storing a transition for each char code for accept state produces less gates than adding an `if` to check if the current state is not "accept"
    // I might be wrong. I tested for input of length 128 and 1024.
    {end_anchor_logic}
    table
}}
      "#
    );

    // If we want to extract a substring, retrieve the state in which the substring will be.
    // *Assumes there is a single substring to be extracted*
    // `substring_ranges` contains the "edges" that transition to and from the substring
    // grab the end (=second part) of the first edge
    let substr_state = regex_and_dfa
        .substrings
        .substring_ranges
        .get(0)
        .and_then(|first_set| first_set.iter().next())
        .map(|&(_, second)| second)
        .unwrap_or(0);

    let fn_body = if gen_substrs {
        format!(
            r#"
  global table = make_lookup_table();
  pub fn regex_match<let N: u32>(input: [u8; N]) -> BoundedVec<Field, N> {{
    // regex: {regex_pattern}
    let mut s = 0;
    let mut substring: BoundedVec<Field, N> = BoundedVec::new();
    s = table[s * 256 + 255 as Field];
    for i in 0..input.len() {{
        let temp = input[i] as Field;
        s = table[s * {BYTE_SIZE} + input[i] as Field];
        if (s == {substr_state}) {{
          substring.push(temp);
      }}
      }}
    assert_eq(s, {accept_state_id}, f"no match: {{s}}");
    substring
  }}
      "#,
            regex_pattern = regex_and_dfa.regex_pattern,
        )
    } else {
        format!(
            r#"
global table = make_lookup_table();
pub fn regex_match<let N: u32>(input: [u8; N]) {{
    // regex: {regex_pattern}
    let mut s = 0;
    s = table[s * 256 + 255 as Field];
    for i in 0..input.len() {{
        s = table[s * {BYTE_SIZE} + input[i] as Field];
    }}
    assert_eq(s, {accept_state_id}, f"no match: {{s}}");
}}
    "#,
            regex_pattern = regex_and_dfa.regex_pattern,
        )
    };
    format!(
        r#"
        {fn_body}
        {lookup_table}
    "#
    )
    .trim()
    .to_owned()
}

fn indent(s: &str) -> String {
    s.split("\n")
        .map(|s| {
            if s.trim().is_empty() {
                s.to_owned()
            } else {
                format!("{}{}", "    ", s)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
