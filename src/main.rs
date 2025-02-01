use clap::{Arg, ArgAction, Command};
use clap_num::maybe_hex;
use prettytable::{cell, row, Table};
use std::process;

// Import our modules/crates.
use exhume_body::Body;
use exhume_extfs::extfs::ExtFS;
use exhume_lvm::Lvm2;

fn main() {
    // Set up Clap command-line argument parsing.
    let matches = Command::new("exhume_lvm")
        .version("1.0")
        .author("Your Name")
        .about("Exhumes and displays LVM and ExtFS partition information")
        .arg(
            Arg::new("body")
                .short('b')
                .long("body")
                .value_parser(clap::value_parser!(String))
                .required(true)
                .help("Path to the partition body file"),
        )
        .arg(
            Arg::new("format")
                .short('f')
                .long("format")
                .value_parser(clap::value_parser!(String))
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
            Arg::new("verbose")
                .short('v')
                .long("verbose")
                .action(ArgAction::SetTrue)
                .help("Enable verbose output"),
        )
        .get_matches();

    // Retrieve the argument values.
    let body_path = matches.get_one::<String>("body").unwrap();
    let format = matches.get_one::<String>("format").unwrap();
    let offset = *matches.get_one::<u64>("offset").unwrap();
    let verbose = *matches.get_one::<bool>("verbose").unwrap_or(&false);

    // Open the "body" using exhume_body.
    let mut body = Body::new_from(body_path.clone(), format, Some(offset));
    if verbose {
        body.print_info();
    }

    // Parse the LVM partition.
    let lvm = match Lvm2::open(&mut body, offset) {
        Ok(lvm) => lvm,
        Err(e) => {
            eprintln!("Error opening LVM partition: {:?}", e);
            process::exit(1);
        }
    };

    // Display LVM information using prettytable.
    print_lvm_info(&lvm);

    // --- Test for ExtFS ---
    // For this example we assume the extfs filesystem is stored inside the first logical volume.
    let lv = match lvm.lvs().next() {
        Some(lv) => lv,
        None => {
            eprintln!("No logical volumes found in the LVM partition");
            process::exit(1);
        }
    };
    println!("Attempting to open ExtFS on logical volume '{}'", lv.name());

    // Open the logical volume (this returns an object implementing Read+Seek).
    let mut lv_reader = lvm.open_lv(lv, &mut body);

    // Attempt to create an extfs object from the logical volume.
    // (Pass 0 as offset because the reader is already positioned at the start of the LV.)
    // let extfs = match ExtFS::new(&mut lv_reader, 0) {
    //     Ok(fs) => fs,
    //     Err(e) => {
    //         eprintln!("Error opening ExtFS: {}", e);
    //         process::exit(1);
    //     }
    // };
    // println!("ExtFS partition detected successfully.");
}

/// Display LVM partition details using prettytable.
fn print_lvm_info(lvm: &Lvm2) {
    // --- Physical Volume ---
    let mut pv_table = Table::new();
    pv_table.add_row(row!["Physical Volume", "PV ID"]);
    pv_table.add_row(row![lvm.pv_name(), lvm.pv_id()]);
    println!("Physical Volume:");
    pv_table.printstd();

    // --- Volume Group ---
    let mut vg_table = Table::new();
    vg_table.add_row(row!["Volume Group", "VG ID", "Extent Size (bytes)"]);
    vg_table.add_row(row![lvm.vg_name(), lvm.vg_id(), lvm.extent_size()]);
    println!("\nVolume Group:");
    vg_table.printstd();

    // --- Logical Volumes ---
    println!("\nLogical Volumes:");
    for lv in lvm.lvs() {
        let mut lv_table = Table::new();
        lv_table.add_row(row!["LV Name", "LV ID", "Size (extents)"]);
        lv_table.add_row(row![lv.name(), lv.id(), lv.size_in_extents()]);
        lv_table.printstd();

        // --- LV Segments ---
        println!("Segments for LV '{}':", lv.name());
        let mut seg_table = Table::new();
        seg_table.add_row(row![
            "Segment Key",
            "Start Extent",
            "Extent Count",
            "Type",
            "Stripe Count",
            "Stripe Size"
        ]);
        for (key, seg) in &lv.raw_metadata().segments.0 {
            seg_table.add_row(row![
                key,
                seg.start_extent,
                seg.extent_count,
                seg.r#type,
                seg.stripe_count
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                seg.stripe_size
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".to_owned())
            ]);
        }
        seg_table.printstd();
        println!("------------------------------------------------------------");
    }
}
