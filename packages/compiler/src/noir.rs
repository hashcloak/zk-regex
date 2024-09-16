use std::{cmp::max, fs::File, io::Write, path::Path};

use itertools::Itertools;

use crate::structs::RegexAndDFA;

const ACCEPT_STATE_ID: &str = "accept";

pub fn gen_noir_fn(regex_and_dfa: &RegexAndDFA, path: &Path) -> Result<(), std::io::Error> {
    let noir_fn = to_noir_fn(regex_and_dfa);
    let mut file = File::create(path)?;
    file.write_all(noir_fn.as_bytes())?;
    file.flush()?;
    Ok(())
}

fn to_noir_fn(regex_and_dfa: &RegexAndDFA) -> String {
    let accept_state_ids = {
        let accept_states = regex_and_dfa
            .dfa
            .states
            .iter()
            .filter(|s| s.state_type == ACCEPT_STATE_ID)
            .map(|s| s.state_id)
            .collect_vec();
        assert!(accept_states.len() > 0, "no accept states");
        accept_states
    };

    const BYTE_SIZE: u32 = 256; // u8 size
    let mut lookup_table_body = String::new();

    // curr_state + char_code -> next_state
    let mut rows: Vec<(usize, u8, usize)> = vec![];


    let mut highest_state = 0;
    for state in regex_and_dfa.dfa.states.iter() {
        for (&tran_next_state_id, tran) in &state.transitions {
            for &char_code in tran {
                rows.push((state.state_id, char_code, tran_next_state_id));
            }
        }
        highest_state = max(state.state_id, highest_state);
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
    
    let mut end_anchor_logic = String::new();
    // If regex_and_dfa.has_end_anchor tells us where the regex ends with `$`
    if regex_and_dfa.has_end_anchor {
      // If so, add transitions from each accept state to invalid state
      // these can be overwritten by valid transitions from accept state further on
      for acc_state in accept_state_ids.clone() {
        end_anchor_logic += 
        &format!(
          r#"
for i in 0..{BYTE_SIZE} {{
    table[{acc_state} * {BYTE_SIZE} + i] = {invalid_state};
}}
            "#
        );
      }
      end_anchor_logic = indent(&end_anchor_logic);
    }

    let lookup_table = format!(
        r#"
comptime fn make_lookup_table() -> [Field; {table_size}] {{
    let mut table = [0; {table_size}];
    {end_anchor_logic}
{lookup_table_body}
    table
}}
      "#
    );

    let final_states_condition_body = accept_state_ids
        .iter()
        .map(|id| format!("(s == {id})"))
        .collect_vec()
        .join(" | ");
    let fn_body = format!(
        r#"
global table = comptime {{ make_lookup_table() }};
pub fn regex_match<let N: u32>(input: [u8; N]) {{
    // regex: {regex_pattern}
    let mut s = 0;
    s = table[s * 256 + 255 as Field];
    for i in 0..input.len() {{
        s = table[s * {BYTE_SIZE} + input[i] as Field];
    }}
    assert({final_states_condition_body}, f"no match: {{s}}");
}}
    "#,
        regex_pattern = regex_and_dfa.regex_pattern,
    );
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
