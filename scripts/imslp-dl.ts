
import { join } from "path";
import { mkdir } from "node:fs/promises";

const WORKS_OUTPUT_DIR = "imslp/works";
const PEOPLE_OUTPUT_DIR = "imslp/people";
const LIMIT = 100;
const BASE_URL = "https://imslp.org/imslpscripts/API.ISCR.php";

async function fetchAndSave(type: number, outputDir: string, typeName: string) {
  await mkdir(outputDir, { recursive: true });

  let start = 0;
  let moreAvailable = true;

  while (moreAvailable) {
    // type=1 for people, type=2 for works
    const url = `${BASE_URL}?account=worklist/disclaimer=accepted/sort=id/type=${type}/start=${start}/limit=${LIMIT}/retformat=json`;
    console.log(`Fetching ${typeName} starting at ${start}...`);

    try {
      const response = await fetch(url);
      if (!response.ok) {
        throw new Error(`HTTP error! status: ${response.status}`);
      }

      const data = await response.json();
      
      const fileName = `${typeName}_${start}.json`;
      const filePath = join(outputDir, fileName);
      
      await Bun.write(filePath, JSON.stringify(data, null, 2));
      console.log(`Saved ${filePath}`);

      if (data.metadata) {
        moreAvailable = data.metadata.moreresultsavailable;
        const itemCount = Object.keys(data).length - 1; 
        if (itemCount <= 0) {
            console.log("No items returned, stopping.");
            moreAvailable = false;
        }
      } else {
        console.warn("No metadata found in response, stopping.");
        moreAvailable = false;
      }

      start += LIMIT;

      await new Promise(resolve => setTimeout(resolve, 500));

    } catch (error) {
      console.error(`Error fetching ${typeName} start=${start}:`, error);
      break;
    }
  }
}

async function main() {
    console.log("Starting People Download...");
    await fetchAndSave(1, PEOPLE_OUTPUT_DIR, "people");
    console.log("People Download Complete.");

    console.log("Starting Works Download...");
    await fetchAndSave(2, WORKS_OUTPUT_DIR, "works");
    console.log("Works Download Complete.");
}

main();
