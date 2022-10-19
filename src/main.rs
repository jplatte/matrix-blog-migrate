use std::{
    env,
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
    process,
};

use anyhow::bail;
use indexmap::IndexMap;
use itertools::Itertools;
use toml::value::Table;
use xshell::{cmd, pushd};

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = env::args().skip(1).collect();
    let input_path = match args.as_slice() {
        [input] => Path::new(input),
        _ => {
            eprintln!("must receive exactly one command line argument (input file)");
            process::exit(1);
        }
    };

    let (date, mut updated) = git_timestamps(input_path)?;
    let mut date = Some(date);
    let (yaml_frontmatter, markdown) = read_file_contents(input_path)?;

    let mut frontmatter_value: IndexMap<String, toml::Value> =
        serde_yaml::from_str(&yaml_frontmatter)?;

    // make sure frontmatter is in the expected format
    for (k, v) in &frontmatter_value {
        match k.as_str() {
            "date" => {
                let v_str = &v.as_str().expect("date must be a string");
                let date_str = &date.as_ref().unwrap();
                if !date_str.starts_with(v_str) {
                    eprintln!(
                        "warning: date mismatch, git date = {date_str}, frontmatter date = {v_str}"
                    );
                    date = None;
                    updated = None;
                }
            }
            "categories" | "author" | "title" => {}
            key => bail!("Unexpected property `{key}`"),
        }
    }

    if let Some(ts) = date {
        frontmatter_value.insert("date".to_owned(), utc_iso_date(ts).into());
    } else {
        frontmatter_value.shift_remove("date");
    }

    if let Some(ts) = updated {
        frontmatter_value.insert("updated".to_owned(), utc_iso_date(ts).into());
    }

    // rewrite frontmatter structure to match Zola's expectations
    convert_taxonomy(&mut frontmatter_value, "author", "author")?;
    convert_taxonomy(&mut frontmatter_value, "categories", "category")?;

    let toml_frontmatter = toml::to_string(&dbg!(frontmatter_value))?;

    println!("+++\n{toml_frontmatter}+++\n{markdown}");

    Ok(())
}

fn git_timestamps(input_path: &Path) -> anyhow::Result<(String, Option<String>)> {
    let _guard = pushd(input_path.parent().expect("input file path has parent"));
    let input_file_name = input_path
        .file_name()
        .expect("input file path has file name");
    let git_file_timestamps =
        cmd!("git log --format=%cd --date=iso-strict -- {input_file_name}").read()?;
    let mut git_file_timestamps = git_file_timestamps.lines();
    let date = git_file_timestamps
        .next()
        .expect("git log command returned at least one line");
    let updated = git_file_timestamps.next_back();

    Ok((date.to_owned(), updated.map(ToOwned::to_owned)))
}

fn read_file_contents(input_path: &Path) -> anyhow::Result<(String, String)> {
    let input = BufReader::new(File::open(input_path)?);
    let mut input_lines = input.lines();

    let first_line = input_lines.next().expect("input file is non-empty")?;
    assert_eq!(first_line, "---", "File must start with YAML frontmatter");

    let mut frontmatter = String::new();
    loop {
        match input_lines.next().transpose()? {
            Some(s) if s == "---" => break,
            Some(s) => {
                frontmatter += s.as_str();
                frontmatter.push('\n');
            }
            None => bail!("Couldn't find end of frontmatter"),
        }
    }

    let markdown = input_lines
        // Okay for I/O errors to panic in this simple script
        .map(|result| result.unwrap())
        .join("\n");

    Ok((frontmatter, markdown))
}

fn convert_taxonomy(
    frontmatter_value: &mut IndexMap<String, toml::Value>,
    old_key: &str,
    new_key: &str,
) -> anyhow::Result<()> {
    if let Some(mut value) = frontmatter_value.remove(old_key) {
        let table = frontmatter_value
            .entry("taxonomies".to_owned())
            .or_insert_with(|| toml::Value::Table(Table::new()))
            .as_table_mut()
            .unwrap();

        match &value {
            toml::Value::String(s) => value = toml::Value::Array(vec![s.to_owned().into()]),
            toml::Value::Array(_) => {}
            _ => bail!("Unexpected value for `{old_key}`: {value:?}"),
        }

        table.insert(new_key.to_owned(), value);
    }

    Ok(())
}

fn utc_iso_date(iso_date: String) -> String {
    cmd!("date --date={iso_date} +%Y-%m-%dT%H:%M:%SZ")
        .read()
        .expect("date conversion works")
}
