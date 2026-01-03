
import { join } from "path";
import { mkdir } from "node:fs/promises";

const OUTPUT_DIR = "imslp";
const LIMIT = 100;
const BASE_URL = "https://imslp.org/imslpscripts/API.ISCR.php";

async function downloadWorks() {
  await mkdir(OUTPUT_DIR, { recursive: true });

  let start = 0;
  let moreAvailable = true;

  while (moreAvailable) {
    const url = `${BASE_URL}?account=worklist/disclaimer=accepted/sort=id/type=2/start=${start}/limit=${LIMIT}/retformat=json`;
    console.log(`Fetching works starting at ${start}...`);

    try {
      const response = await fetch(url);
      if (!response.ok) {
        throw new Error(`HTTP error! status: ${response.status}`);
      }

      const data = await response.json();
      
      // The API returns an object where keys are indices ("0", "1", etc.) and "metadata"
      // We want to save the whole response to preserve the structure and metadata
      const fileName = `works_${start}.json`;
      const filePath = join(OUTPUT_DIR, fileName);
      
      await Bun.write(filePath, JSON.stringify(data, null, 2));
      console.log(`Saved ${filePath}`);

      if (data.metadata) {
        moreAvailable = data.metadata.moreresultsavailable;
        // Also check if we actually got items to avoid infinite loops in case of API weirdness
        // data.metadata is one key.
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

      // Be polite to the API
      await new Promise(resolve => setTimeout(resolve, 500));

    } catch (error) {
      console.error(`Error fetching start=${start}:`, error);
      // Retry logic could be added here, but for a simple script we might just stop or skip
      // For now, let's break to avoid spamming errors
      break;
    }
  }

  console.log("Download complete.");
}

downloadWorks();
