use clap::{value_parser, Arg, Command};
use clap_num::maybe_hex;
use exhume_body::{Body, BodySlice};
use exhume_lvm::Lvm2;
use log::{debug, error, info};
use prettytable::{Cell, Row, Table};
use std::process;

fn main() {
    let matches = Command::new("exhume_lvm")
        .version("1.0")
        .author("ForensicXlab")
        .about("Exhumes and displays LVM information")
        .arg(
            Arg::new("body")
                .short('b')
                .long("body")
                .value_parser(value_parser!(String))
                .required(true)
                .help("Path to the partition body file"),
        )
        .arg(
            Arg::new("format")
                .short('f')
                .long("format")
                .value_parser(value_parser!(String))
                .required(true)
                .help("File format: either 'raw' or 'ewf'"),
        )
        .arg(
            Arg::new("offset")
                .short('o')
                .long("offset")
                .value_parser(maybe_hex::<u64>)
                .required(true)
                .help("LVM partition starts at address 0x..."),
        )
        .arg(
            Arg::new("size")
                .short('s')
                .long("size")
                .value_parser(maybe_hex::<u64>)
                .required(true)
                .help("LVM partition size."),
        )
        .arg(
            Arg::new("log_level")
                .short('l')
                .long("log-level")
                .value_parser(["error", "warn", "info", "debug", "trace"])
                .default_value("info")
                .help("Set the log verbosity level"),
        )
        .get_matches();

    // Initialize logger.
    let log_level_str = matches.get_one::<String>("log_level").unwrap();
    let level_filter = match log_level_str.as_str() {
        "error" => log::LevelFilter::Error,
        "warn" => log::LevelFilter::Warn,
        "info" => log::LevelFilter::Info,
        "debug" => log::LevelFilter::Debug,
        "trace" => log::LevelFilter::Trace,
        _ => log::LevelFilter::Info,
    };
    env_logger::Builder::new().filter_level(level_filter).init();

    let body_path = matches.get_one::<String>("body").unwrap();
    let format = matches.get_one::<String>("format").unwrap();
    let offset = *matches.get_one::<u64>("offset").unwrap();

    let mut body = Body::new(body_path.clone(), format);

    let size = *matches.get_one::<u64>("size").unwrap() * body.get_sector_size() as u64;

    let mut partition = BodySlice::new(&mut body, offset, size).unwrap();
    debug!("Created Body from '{}'", body_path);

    let lvm = match Lvm2::open(&mut partition) {
        Ok(lvm) => lvm,
        Err(e) => {
            error!("Error opening LVM partition: {:?}", e);
            process::exit(1);
        }
    };

    print_lvm_info(&lvm);
}

/// Instead of printing directly to stdout, we capture the table output
/// and log it at the info level.
fn print_lvm_info(lvm: &Lvm2) {
    let mut table = Table::new();

    // Header row.
    table.add_row(Row::new(vec![
        Cell::new("Physical Volume"),
        Cell::new("Volume Group"),
        Cell::new("Logical Volume"),
        Cell::new("Segment"),
    ]));

    let pv_info = format!("Name: {}\nID: {}", lvm.pv_name(), lvm.pv_id());
    let vg_info = format!(
        "Name: {}\nID: {}\nExtent Size: {}",
        lvm.vg_name(),
        lvm.vg_id(),
        lvm.extent_size()
    );

    for lv in lvm.lvs() {
        let lv_info = format!(
            "Name: {}\nID: {}\nSize (extents): {}",
            lv.name(),
            lv.id(),
            lv.size_in_extents()
        );
        if lv.raw_metadata().segments.0.is_empty() {
            table.add_row(Row::new(vec![
                Cell::new(&pv_info),
                Cell::new(&vg_info),
                Cell::new(&lv_info),
                Cell::new("No segments"),
            ]));
        } else {
            for (seg_key, seg) in &lv.raw_metadata().segments.0 {
                let seg_info =
                    format!(
                    "Key: {}\nStart: {}\nCount: {}\nType: {}\nStripe Count: {}\nStripe Size: {}",
                    seg_key,
                    seg.start_extent,
                    seg.extent_count,
                    seg.r#type,
                    seg.stripe_count.map(|n| n.to_string()).unwrap_or_else(|| "-".to_owned()),
                    seg.stripe_size.map(|n| n.to_string()).unwrap_or_else(|| "-".to_owned()),
                );
                table.add_row(Row::new(vec![
                    Cell::new(&pv_info),
                    Cell::new(&vg_info),
                    Cell::new(&lv_info),
                    Cell::new(&seg_info),
                ]));
            }
        }
    }
    table.printstd()
}
