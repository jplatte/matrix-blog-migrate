use std::{
    env,
    fs::{self, File},
    io::{BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    process,
};

use anyhow::bail;
use heck::ToKebabCase;
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

    assert!(input_path.ends_with(".mdx"));

    let (date, mut updated) = git_timestamps(input_path)?;
    let mut date = Some(date);
    let (yaml_frontmatter, markdown) = read_file_contents(input_path)?;

    let mut frontmatter_value: IndexMap<String, toml::Value> =
        serde_yaml::from_str(&yaml_frontmatter)?;

    let frontmatter_date = match frontmatter_value
        .shift_remove("date")
        .expect("all pages have a date")
    {
        toml::Value::String(s) => s,
        _ => bail!("frontmatter date is not a string"),
    };

    {
        let date_str = date.as_ref().unwrap();
        if !date_str.starts_with(&frontmatter_date) {
            eprintln!(
            "warning: date mismatch, git date = {date_str}, frontmatter date = {frontmatter_date}"
        );
            date = None;
            updated = None;
        }
    }

    let title = frontmatter_value["title"]
        .as_str()
        .expect("title must be a string")
        .to_owned();

    let slug = match frontmatter_value.shift_remove("slug") {
        Some(toml::Value::String(s)) => s,
        Some(_) => bail!("slug must be a string"),
        None => title.to_kebab_case(),
    };

    // check for unexpected frontmatter fields
    for k in frontmatter_value.keys() {
        match k.as_str() {
            "categories" | "author" | "title" => {}
            key => bail!("Unexpected frontmatter field `{key}`"),
        }
    }

    assert!(frontmatter_date.is_ascii());
    assert_eq!(frontmatter_date.len(), "yyyy-mm-dd".len());
    assert_eq!(frontmatter_date.as_bytes()[4], b'-');
    assert_eq!(frontmatter_date.as_bytes()[7], b'-');
    let year = &frontmatter_date[..4];
    let month = &frontmatter_date[5..7];
    let day = &frontmatter_date[8..];

    if let Some(ts) = date {
        frontmatter_value.insert("date".to_owned(), utc_iso_date(ts).into());
    }

    if let Some(ts) = updated {
        frontmatter_value.insert("updated".to_owned(), utc_iso_date(ts).into());
    }

    frontmatter_value.insert(
        "path".to_owned(),
        format!("/blog/{year}/{month}/{day}/{slug}").into(),
    );

    // rewrite frontmatter structure to match Zola's expectations
    convert_taxonomy(&mut frontmatter_value, "author", "author")?;
    convert_taxonomy(&mut frontmatter_value, "categories", "category")?;

    let toml_frontmatter = toml::to_string(&frontmatter_value)?;

    let output_path = PathBuf::from(format!("{year}/{month}/{year}-{month}-{day}-{slug}.md"));
    fs::create_dir_all(output_path.parent().unwrap())?;

    let mut writer = BufWriter::new(File::create(output_path)?);
    writeln!(writer, "+++\n{toml_frontmatter}+++\n{markdown}")?;

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
